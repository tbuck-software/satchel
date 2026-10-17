// Generated from Rust. Run pnpm ipc:generate.
import type { WorkflowState } from "../../workflow/states/WorkflowState";
export type ThreadRecord = {
    id: string;
    account_id: string;
    subject: string;
    snippet: string;
    last_message_date: string;
    /**
     * Stored keyset date, distinct from the latest displayed message date.
     */
    sort_date: string | null;
    message_count: number;
    workflow_state: WorkflowState;
    unread_count: number;
    has_attachments: boolean;
    has_draft: boolean;
    draft_snippet: string | null;
    priority: number;
    board_position: number | null;
    tags: Array<string>;
    assignee: string | null;
    shared_with: Array<string>;
    participants: string;
    sent_participants: string;
    mailbox_names: string;
    category: string | null;
    /**
     * Same classification used by smart-folder summaries and list filters.
     */
    smart_bucket: string;
    is_muted: boolean;
    is_starred: boolean;
    snooze_until: string | null;
    sender_known: boolean;
    has_invite: boolean;
    invite_status: string | null;
    invite_cancelled: boolean;
};
