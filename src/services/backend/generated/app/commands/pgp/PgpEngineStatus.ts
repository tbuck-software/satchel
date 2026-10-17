// Generated from Rust. Run pnpm ipc:generate.
export type PgpEngineStatus = {
    /**
     * True when the selected engine is callable.
     */
    available: boolean;
    /**
     * Engine identifier, either "rpgp" or "gpg".
     */
    engine: string;
    can_decrypt: boolean;
    can_sign: boolean;
    can_encrypt: boolean;
};
