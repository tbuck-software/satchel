// Developer diagnostics for the mock transport; excluded from production bundles.
import manifest from './generated/command-manifest';

type Shape =
  | { kind: 'value' }
  | { kind: 'optional' | 'array'; item: Shape }
  | { kind: 'object'; fields: Record<string, { required: boolean; shape: Shape }> }
  | { kind: 'ref'; name: string };

const commands: Record<string, Shape> = manifest.commands;
const types: Record<string, Shape> = manifest.types;

function validate(shape: Shape, value: unknown, path: string): void {
  switch (shape.kind) {
    case 'value':
      return;
    case 'optional':
      if (value != null) validate(shape.item, value, path);
      return;
    case 'array':
      if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
      value.forEach((item, index) => validate(shape.item, item, `${path}[${index}]`));
      return;
    case 'ref': {
      const type = types[shape.name];
      if (!type) throw new Error(`unknown input type ${shape.name}`);
      validate(type, value, path);
      return;
    }
    case 'object': {
      if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        throw new Error(`${path} must be an object`);
      }
      const input = value as Record<string, unknown>;
      for (const key of Object.keys(input)) {
        if (!Object.hasOwn(shape.fields, key)) throw new Error(`${path}.${key} is not a Rust argument`);
      }
      for (const [key, field] of Object.entries(shape.fields)) {
        if (input[key] === undefined) {
          if (field.required) throw new Error(`${path}.${key} is required by Rust`);
        } else {
          validate(field.shape, input[key], `${path}.${key}`);
        }
      }
    }
  }
}

/** Check wire argument names before any mock handler can accept them. */
export function assertCommandPayload(command: string, payload: Record<string, unknown> = {}): void {
  if (!Object.hasOwn(commands, command)) throw new Error(`unknown Rust command ${command}`);
  const shape = commands[command];
  if (!shape) throw new Error(`unknown Rust command ${command}`);
  validate(shape, payload, command);
}
