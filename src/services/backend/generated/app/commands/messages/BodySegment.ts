// Generated from Rust. Run pnpm ipc:generate.
/**
 * Transient content provenance. Never reconstructed from persisted body text.
 */
export type BodySegment = {
    "kind": "text";
    "content": string;
} | {
    "kind": "pgp_decrypted";
    "content": string;
} | {
    "kind": "smime_decrypted";
    "content": string;
} | {
    "kind": "pgp_undecrypted";
    "content": string;
};
