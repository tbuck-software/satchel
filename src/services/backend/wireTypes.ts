import type manifest from './generated/command-manifest';

/** Older cached rows and frontend fixtures may omit fields handled with defaults. */
export type OptionalWireFields<T, K extends keyof T> = Omit<T, K> & Partial<Pick<T, K>>;

/** Narrow domain values while retaining the generated field types. */
export type RefinedWireFields<T, R extends Partial<T>> =
  Exclude<keyof R, keyof T> extends never ? Omit<T, keyof R> & R : never;

/** Wider JSON and cached-row views; their exceptions are listed in docs/IPC.md. */
export type ProjectedWireFields<T, R extends Partial<Record<keyof T, unknown>>> =
  Exclude<keyof R, keyof T> extends never ? Omit<T, keyof R> & R : never;

type InputTypes = typeof manifest.types;
type InputField<T, S> = S extends { kind: 'ref'; name: infer N extends keyof InputTypes }
  ? InputField<T, InputTypes[N]>
  : S extends { kind: 'optional'; item: infer I }
    ? InputField<NonNullable<T>, I> | null
    : S extends { kind: 'array'; item: infer I }
      ? T extends readonly (infer E)[] ? InputField<E, I>[] : T
      : S extends { kind: 'object'; fields: infer F }
        ? InputObject<T, F>
        : T;

type InputObject<T, F> = {
  [K in keyof T as K extends keyof F ? F[K] extends { required: false } ? never : K : K]-?:
    K extends keyof F ? F[K] extends { shape: infer S } ? InputField<T[K], S> : T[K] : T[K];
} & {
  [K in keyof T as K extends keyof F ? F[K] extends { required: false } ? K : never : never]?:
    K extends keyof F ? F[K] extends { shape: infer S } ? InputField<T[K], S> : T[K] : T[K];
};

/** Serde permits omitted default fields, including inside arrays of inputs. */
export type DeserializeWireInput<T, N extends keyof InputTypes> = InputField<T, InputTypes[N]>;
