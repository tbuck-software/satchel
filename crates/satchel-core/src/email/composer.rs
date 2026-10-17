use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingDraft {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

pub fn compose_rfc822(draft: &OutgoingDraft) -> String {
    format!(
        "To: {}\nCc: {}\nBcc: {}\nSubject: {}\n\n{}",
        draft.to.join(", "),
        draft.cc.join(", "),
        draft.bcc.join(", "),
        draft.subject,
        draft.body
    )
}
