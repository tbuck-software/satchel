// Generated from Rust. Run pnpm ipc:generate.
/**
 * An existing event open in the editor: its own time does not count against it.
 */
export type EditingEventInput = {
    event_id: string;
    /**
     * The saved slot, RFC 3339. Attendee busy time exactly this slot is the event.
     */
    starts_at?: string | null;
    ends_at?: string | null;
};
