//! Read command signatures and serde field names without loading the application.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use heck::{ToLowerCamelCase, ToSnakeCase};
use quote::ToTokens;
use serde::Serialize;
use syn::ext::IdentExt;
use syn::{Attribute, Fields, FnArg, GenericArgument, Item, Pat, PathArguments, Type};

fn has_list_item(attrs: &[Attribute], list_name: &str, name: &str) -> bool {
    fn in_meta(meta: &syn::Meta, list_name: &str, name: &str) -> bool {
        let syn::Meta::List(list) = meta else {
            return false;
        };
        if list.path.is_ident(list_name) {
            let items = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .unwrap();
            return items.iter().any(|item| {
                item.path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == name)
            });
        }
        if list.path.is_ident("cfg_attr") {
            let items = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .unwrap();
            return items
                .iter()
                .skip(1)
                .any(|meta| in_meta(meta, list_name, name));
        }
        false
    }
    attrs
        .iter()
        .any(|attr| in_meta(&attr.meta, list_name, name))
}

fn has_derive(attrs: &[Attribute], name: &str) -> bool {
    has_list_item(attrs, "derive", name)
}

fn check_ts_export(attrs: &[Attribute], name: &str) {
    assert!(has_derive(attrs, "TS"), "missing TS derive: {name}");
    assert!(
        has_list_item(attrs, "ts", "export"),
        "missing ts(export): {name}"
    );
}

#[derive(Default)]
struct SerdeAttrs {
    rename: Option<String>,
    rename_all: Option<String>,
    default: bool,
    skip: bool,
}

fn serde_attrs(attrs: &[Attribute]) -> SerdeAttrs {
    let mut result = SerdeAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                result.rename = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("rename_all") {
                result.rename_all = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else {
                result.default |= meta.path.is_ident("default");
                result.skip |=
                    meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing");
                if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                } else if meta.input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let _: proc_macro2::TokenStream = content.parse()?;
                }
            }
            Ok(())
        })
        .expect("supported serde metadata");
    }
    result
}

// Serde field casing differs from heck's command-argument casing.
fn field_name(name: &str, casing: Option<&str>) -> String {
    match casing {
        None | Some("lowercase" | "snake_case") => name.into(),
        Some("UPPERCASE" | "SCREAMING_SNAKE_CASE") => name.to_ascii_uppercase(),
        Some("PascalCase" | "camelCase") => {
            let mut result = String::new();
            let mut capitalize = true;
            for character in name.chars() {
                if character == '_' {
                    capitalize = true;
                } else {
                    result.push(if capitalize {
                        character.to_ascii_uppercase()
                    } else {
                        character
                    });
                    capitalize = false;
                }
            }
            if casing == Some("camelCase") {
                result[..1].make_ascii_lowercase();
            }
            result
        }
        Some("kebab-case") => name.replace('_', "-"),
        Some("SCREAMING-KEBAB-CASE") => name.to_ascii_uppercase().replace('_', "-"),
        Some(other) => panic!("unsupported serde casing {other}"),
    }
}

fn argument_name(name: &str, casing: &str) -> String {
    match casing {
        "camelCase" => name.to_lower_camel_case(),
        "snake_case" => name.to_snake_case(),
        other => panic!("unsupported command casing {other}"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Shape {
    Value,
    Optional { item: Box<Shape> },
    Array { item: Box<Shape> },
    Object { fields: BTreeMap<String, Field> },
    Ref { name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Field {
    required: bool,
    shape: Shape,
}

#[derive(Clone)]
struct Definition {
    module: String,
    fields: Fields,
    attrs: Vec<Attribute>,
    is_enum: bool,
}

#[derive(Default)]
struct Catalog {
    definitions: BTreeMap<String, Definition>,
    commands: Vec<(String, syn::ItemFn)>,
    imports: BTreeMap<String, BTreeMap<String, String>>,
}

fn flatten_use(tree: &syn::UseTree, prefix: &str, result: &mut BTreeMap<String, String>) {
    match tree {
        syn::UseTree::Path(path) => {
            flatten_use(&path.tree, &format!("{prefix}{}::", path.ident), result)
        }
        syn::UseTree::Name(name) => {
            result.insert(name.ident.to_string(), format!("{prefix}{}", name.ident));
        }
        syn::UseTree::Rename(rename) => {
            result.insert(
                rename.rename.to_string(),
                format!("{prefix}{}", rename.ident),
            );
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix, result);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

impl Catalog {
    fn collect(&mut self, module: &str, items: Vec<Item>) {
        for item in items {
            match item {
                Item::Struct(item) => {
                    if module.starts_with("app::commands::")
                        && matches!(item.vis, syn::Visibility::Public(_))
                        && (has_derive(&item.attrs, "Serialize")
                            || has_derive(&item.attrs, "Deserialize"))
                    {
                        check_ts_export(&item.attrs, &format!("{module}::{}", item.ident));
                    }
                    self.definitions.insert(
                        format!("{module}::{}", item.ident),
                        Definition {
                            module: module.into(),
                            fields: item.fields,
                            attrs: item.attrs,
                            is_enum: false,
                        },
                    );
                }
                Item::Enum(item) => {
                    if module.starts_with("app::commands::")
                        && matches!(item.vis, syn::Visibility::Public(_))
                        && (has_derive(&item.attrs, "Serialize")
                            || has_derive(&item.attrs, "Deserialize"))
                    {
                        check_ts_export(&item.attrs, &format!("{module}::{}", item.ident));
                    }
                    self.definitions.insert(
                        format!("{module}::{}", item.ident),
                        Definition {
                            module: module.into(),
                            fields: Fields::Unit,
                            attrs: item.attrs,
                            is_enum: true,
                        },
                    );
                }
                Item::Fn(item)
                    if item.attrs.iter().any(|attr| {
                        attr.path()
                            .segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect::<Vec<_>>()
                            == ["tauri", "command"]
                    }) =>
                {
                    self.commands.push((module.into(), item))
                }
                Item::Use(item) => flatten_use(
                    &item.tree,
                    "",
                    self.imports.entry(module.into()).or_default(),
                ),
                Item::Mod(item) if item.ident != "tests" => {
                    if let Some((_, items)) = item.content {
                        self.collect(&format!("{module}::{}", item.ident), items);
                    }
                }
                _ => {}
            }
        }
    }

    fn resolve(&self, module: &str, path: &syn::Path) -> Option<String> {
        let parts = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        let name = parts.last()?;
        let local = format!("{module}::{name}");
        if parts.len() == 1 && self.definitions.contains_key(&local) {
            return Some(local);
        }
        let mut qualified = parts.join("::");
        if let Some(import) = self
            .imports
            .get(module)
            .and_then(|imports| imports.get(&parts[0]))
        {
            qualified = format!(
                "{import}{}",
                parts[1..]
                    .iter()
                    .map(|part| format!("::{part}"))
                    .collect::<String>()
            );
        }
        if qualified.starts_with("crate::") {
            qualified = qualified.replacen("crate", module.split("::").next().unwrap(), 1);
        }
        if qualified.starts_with("satchel_core::") {
            qualified = qualified.replacen("satchel_core", "core", 1);
        }
        if qualified.starts_with("super::") {
            qualified = qualified.replacen("super", module.rsplit_once("::").unwrap().0, 1);
        }
        if self.definitions.contains_key(&qualified) {
            return Some(qualified);
        }
        // The shell re-exports core modules, preserving their paths.
        if qualified.starts_with("app::") {
            let core = qualified.replacen("app", "core", 1);
            if self.definitions.contains_key(&core) {
                return Some(core);
            }
        }
        let candidates = self
            .definitions
            .keys()
            .filter(|key| key.ends_with(&format!("::{name}")))
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [only] => Some((*only).clone()),
            [] => None,
            _ => panic!("ambiguous type {module}::{qualified}"),
        }
    }

    fn shape(&self, module: &str, ty: &Type) -> Shape {
        match ty {
            Type::Reference(reference) => self.shape(module, &reference.elem),
            Type::Slice(slice) => Shape::Array {
                item: Box::new(self.shape(module, &slice.elem)),
            },
            Type::Array(array) => Shape::Array {
                item: Box::new(self.shape(module, &array.elem)),
            },
            Type::Path(path) => {
                let segment = path.path.segments.last().unwrap();
                let item = match &segment.arguments {
                    PathArguments::AngleBracketed(args) => {
                        args.args.iter().find_map(|arg| match arg {
                            GenericArgument::Type(ty) => Some(ty),
                            _ => None,
                        })
                    }
                    _ => None,
                };
                match (segment.ident.to_string().as_str(), item) {
                    ("Option", Some(item)) => Shape::Optional {
                        item: Box::new(self.shape(module, item)),
                    },
                    ("Vec" | "HashSet" | "BTreeSet", Some(item)) => Shape::Array {
                        item: Box::new(self.shape(module, item)),
                    },
                    _ => self
                        .resolve(module, &path.path)
                        .map_or(Shape::Value, |name| {
                            if self.definitions[&name].is_enum {
                                Shape::Value
                            } else {
                                Shape::Ref { name }
                            }
                        }),
                }
            }
            _ => Shape::Value,
        }
    }

    fn fields(&self, definition: &Definition) -> BTreeMap<String, Field> {
        let container = serde_attrs(&definition.attrs);
        definition
            .fields
            .iter()
            .filter_map(|field| {
                let attrs = serde_attrs(&field.attrs);
                if attrs.skip {
                    return None;
                }
                let name = attrs.rename.unwrap_or_else(|| {
                    field_name(
                        &field
                            .ident
                            .as_ref()
                            .expect("named wire field")
                            .unraw()
                            .to_string(),
                        container.rename_all.as_deref(),
                    )
                });
                let shape = self.shape(&definition.module, &field.ty);
                let required = !(container.default
                    || attrs.default
                    || matches!(shape, Shape::Optional { .. }));
                Some((name, Field { required, shape }))
            })
            .collect()
    }

    fn check_exported(&self, module: &str, ty: &Type) {
        match ty {
            Type::Path(path) => {
                if let Some(name) = self.resolve(module, &path.path) {
                    check_ts_export(&self.definitions[&name].attrs, &name);
                }
                for segment in &path.path.segments {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        for arg in &args.args {
                            if let GenericArgument::Type(ty) = arg {
                                self.check_exported(module, ty);
                            }
                        }
                    }
                }
            }
            Type::Tuple(tuple) => {
                for ty in &tuple.elems {
                    self.check_exported(module, ty);
                }
            }
            Type::Reference(reference) => self.check_exported(module, &reference.elem),
            _ => {}
        }
    }

    fn expanded(&self, shape: &Shape) -> Shape {
        match shape {
            Shape::Ref { name } => {
                let fields = self.fields(&self.definitions[name]);
                self.expanded(&Shape::Object { fields })
            }
            Shape::Object { fields } => Shape::Object {
                fields: fields
                    .iter()
                    .map(|(name, field)| {
                        (
                            name.clone(),
                            Field {
                                required: field.required,
                                shape: self.expanded(&field.shape),
                            },
                        )
                    })
                    .collect(),
            },
            Shape::Optional { item } => Shape::Optional {
                item: Box::new(self.expanded(item)),
            },
            Shape::Array { item } => Shape::Array {
                item: Box::new(self.expanded(item)),
            },
            Shape::Value => Shape::Value,
        }
    }

    fn manifest(&self) -> Manifest {
        let mut commands = BTreeMap::new();
        for (module, command) in &self.commands {
            if let syn::ReturnType::Type(_, ty) = &command.sig.output {
                self.check_exported(module, ty);
            }
            let mut casing = "camelCase".to_string();
            for attr in &command.attrs {
                if attr
                    .path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "command")
                    && matches!(attr.meta, syn::Meta::List(_))
                {
                    attr.parse_nested_meta(|meta| {
                        if meta.path.is_ident("rename_all") {
                            casing = meta.value()?.parse::<syn::LitStr>()?.value();
                        }
                        Ok(())
                    })
                    .expect("command attributes");
                }
            }
            let mut fields = BTreeMap::new();
            for arg in &command.sig.inputs {
                let FnArg::Typed(arg) = arg else {
                    panic!("command receiver");
                };
                let Type::Path(ty) = &*arg.ty else {
                    panic!(
                        "command argument must be a path: {}",
                        arg.ty.to_token_stream()
                    );
                };
                let last = ty.path.segments.last().unwrap().ident.to_string();
                if [
                    "State",
                    "AppHandle",
                    "Window",
                    "WebviewWindow",
                    "Webview",
                    "Request",
                ]
                .contains(&last.as_str())
                {
                    continue;
                }
                self.check_exported(module, &arg.ty);
                let Pat::Ident(pat) = &*arg.pat else {
                    panic!("command argument must be named");
                };
                let name = argument_name(&pat.ident.unraw().to_string(), &casing);
                let shape = self.shape(module, &arg.ty);
                fields.insert(
                    name,
                    Field {
                        required: !matches!(shape, Shape::Optional { .. }),
                        shape,
                    },
                );
            }
            let shape = Shape::Object { fields };
            if let Some(previous) = commands.get(&command.sig.ident.to_string()) {
                assert_eq!(
                    self.expanded(previous),
                    self.expanded(&shape),
                    "platform command signatures differ: {}",
                    command.sig.ident
                );
            } else {
                commands.insert(command.sig.ident.to_string(), shape);
            }
        }
        let mut types = BTreeMap::new();
        let mut pending = commands.values().cloned().collect::<Vec<_>>();
        while let Some(shape) = pending.pop() {
            match shape {
                Shape::Ref { name } if !types.contains_key(&name) => {
                    let definition = &self.definitions[&name];
                    let shape = Shape::Object {
                        fields: self.fields(definition),
                    };
                    types.insert(name, shape.clone());
                    pending.push(shape);
                }
                Shape::Object { fields } => {
                    pending.extend(fields.into_values().map(|field| field.shape))
                }
                Shape::Optional { item } | Shape::Array { item } => pending.push(*item),
                _ => {}
            }
        }
        Manifest { commands, types }
    }
}

#[derive(Serialize)]
struct Manifest {
    commands: BTreeMap<String, Shape>,
    types: BTreeMap<String, Shape>,
}

fn scan(root: &Path, directory: &Path, namespace: &str, catalog: &mut Catalog) {
    let mut entries = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            scan(root, &path, namespace, catalog);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let relative = path.strip_prefix(root).unwrap().with_extension("");
            let mut parts = relative
                .iter()
                .map(|part| part.to_str().unwrap())
                .collect::<Vec<_>>();
            if ["mod", "lib", "main"].contains(parts.last().unwrap()) {
                parts.pop();
            }
            if parts
                .iter()
                .any(|part| *part == "tests" || part.ends_with("_tests"))
            {
                continue;
            }
            let module = std::iter::once(namespace)
                .chain(parts)
                .collect::<Vec<_>>()
                .join("::");
            catalog.collect(
                &module,
                syn::parse_file(&fs::read_to_string(path).unwrap())
                    .unwrap()
                    .items,
            );
        }
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut catalog = Catalog::default();
    for (path, namespace) in [
        ("src-tauri/src", "app"),
        ("crates/satchel-core/src", "core"),
    ] {
        let path = root.join(path);
        scan(&path, &path, namespace, &mut catalog);
    }
    let manifest = catalog.manifest();
    let output = format!("// Generated from Rust command signatures. Run pnpm ipc:generate.\nexport default {} as const;\n", serde_json::to_string_pretty(&manifest).unwrap());
    let out = std::env::args().nth(1).expect("output directory");
    fs::create_dir_all(&out).unwrap();
    fs::write(Path::new(&out).join("command-manifest.ts"), output).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str) -> Manifest {
        let mut catalog = Catalog::default();
        catalog.collect("app::fixture", syn::parse_file(source).unwrap().items);
        catalog.manifest()
    }

    #[test]
    fn signatures_preserve_nested_serde_names_and_skip_injected_arguments() {
        let manifest = fixture(
            r#"
            #[derive(Deserialize, ts_rs::TS)]
            #[ts(export)]
            #[serde(rename_all = "camelCase")]
            struct Input {
                thread_id: String,
                #[serde(default, rename = "other_ids")]
                other_threads: Vec<Entry>,
                at: Option<String>,
            }
            #[derive(ts_rs::TS)] #[ts(export)] struct Entry { message_id: String }
            #[tauri::command]
            fn update(_app: AppHandle, state: State<AppState>, input: Input, thread_id: String) {}
        "#,
        );
        let Shape::Object { fields } = &manifest.commands["update"] else {
            panic!()
        };
        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["input", "threadId"]
        );
        let Shape::Object { fields } = &manifest.types["app::fixture::Input"] else {
            panic!()
        };
        assert!(fields["threadId"].required);
        assert!(!fields["other_ids"].required);
        assert!(!fields["at"].required);
        assert!(manifest.types.contains_key("app::fixture::Entry"));
    }

    #[test]
    fn explicit_snake_case_and_leading_underscore_match_tauri() {
        let manifest = fixture(
            r#"
            #[tauri::command(rename_all = "snake_case")]
            fn update(_thread_id: String) {}
        "#,
        );
        let Shape::Object { fields } = &manifest.commands["update"] else {
            panic!()
        };
        assert!(fields.contains_key("thread_id"));
    }

    #[test]
    fn raw_identifiers_use_their_wire_names() {
        let manifest = fixture(
            r#"
            #[derive(ts_rs::TS)]
            #[ts(export)]
            struct Input { r#type: String }
            #[tauri::command]
            fn update(r#type: String, input: Input) {}
        "#,
        );
        let Shape::Object { fields } = &manifest.commands["update"] else {
            panic!()
        };
        assert!(fields.contains_key("type"));
        let Shape::Object { fields } = &manifest.types["app::fixture::Input"] else {
            panic!()
        };
        assert!(fields.contains_key("type"));
    }

    #[test]
    #[should_panic(expected = "missing TS derive")]
    fn new_public_command_dtos_require_an_export() {
        let mut catalog = Catalog::default();
        catalog.collect(
            "app::commands::fixture",
            syn::parse_file(
                r#"
            #[derive(Deserialize)]
            pub struct NewInput { id: String }
        "#,
            )
            .unwrap()
            .items,
        );
    }

    #[test]
    fn serde_field_casing_preserves_repeated_underscores_and_acronyms() {
        assert_eq!(field_name("message__id", Some("camelCase")), "messageId");
        assert_eq!(field_name("_message_id", Some("snake_case")), "_message_id");
        assert_eq!(field_name("messageURL", Some("camelCase")), "messageURL");
        assert_eq!(field_name("message_id", Some("lowercase")), "message_id");
        assert_eq!(
            field_name("message_id", Some("SCREAMING-KEBAB-CASE")),
            "MESSAGE-ID"
        );
    }

    #[test]
    #[should_panic(expected = "missing ts(export)")]
    fn a_ts_derive_without_an_export_cannot_leave_the_contract_stale() {
        fixture(
            r#"
            #[derive(Deserialize, ts_rs::TS)]
            struct Input { id: String }
            #[tauri::command]
            fn update(input: Input) {}
        "#,
        );
    }

    #[test]
    #[should_panic(expected = "platform command signatures differ")]
    fn desktop_and_mobile_inputs_must_have_the_same_wire_shape() {
        fixture(
            r#"
            #[tauri::command] fn update(thread_id: String) {}
            mod mobile {
                #[tauri::command] fn update(message_id: String) {}
            }
        "#,
        );
    }
}
