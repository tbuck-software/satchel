// Generated from Rust. Run pnpm ipc:generate.
export type NotificationPreferences = {
    /**
     * Master toggle.
     */
    enabled: boolean;
    /**
     * "all" | "inbox" | "off"
     */
    scope: string;
    sound: boolean;
    /**
     * Number of seconds the user has to undo a send before the network call fires.
     * 0 disables Undo Send.
     */
    undo_send_seconds: number;
    /**
     * Number of days a trashed thread stays around before the maintenance worker
     * hard-deletes it. 0 disables auto-cleanup.
     */
    trash_auto_delete_days: number;
    /**
     * Whether to add a `Disposition-Notification-To` header to outgoing mail
     * requesting a read receipt. Off by default for privacy.
     */
    send_read_receipts: boolean;
    /**
     * How many days of mail are stored with their body. 0 keeps everything.
     *
     * Older mail keeps its sender, subject, date and place in the thread — all
     * of that comes from the header — and fetches its body when opened. A
     * phone defaults to a month — what Gmail and Outlook Mobile both do —
     * because a mailbox stored whole costs what a mailbox costs; a desktop
     * keeps everything, which is what offline search over years of mail rests
     * on.
     */
    body_window_days: number;
};
