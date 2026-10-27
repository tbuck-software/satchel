//! Message read/search queries and their Tauri-free input/output types.
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use std::{collections::HashMap, time::Instant};
use thiserror::Error;

use crate::db::data_repairs;
use crate::db::threads::{push_thread_scope_filters, FolderLookup, ThreadListScope};
use crate::utils::mime::{decode_mime_encoded_words, repair_likely_utf8_mojibake};

mod reads;
mod sent;
mod signatures;

pub use reads::{
    MessagePreviewRecord, MessageReadError, MessageReadRecord, MessageReadRepository,
    MessageSourceRecord,
};
pub use sent::{
    ReplyThreadContext, SentAttachmentMetadata, SentMessageError, SentMessageRecord,
    SentMessageRepository, SentMessageWriteRequest,
};
pub use signatures::{SignatureHistoryMessage, SignatureMessageError, SignatureMessageRepository};

#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/messages/", optional_fields = nullable))]
pub struct SearchQuery {
    pub query: String,
    /// Restrict to a single account. `None` searches every active account
    /// (the unified-inbox case).
    #[serde(default)]
    pub account_id: Option<String>,
    /// Result cap. Clamped to 1..=500; defaults to 50.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Continue after the final hit of the previous page.
    #[serde(default)]
    pub cursor: Option<SearchCursor>,
    /// Keep hits whose thread is in this list, tested the way the thread
    /// list tests it. The list search uses it to search the list on screen.
    #[serde(default)]
    pub list_scope: Option<ThreadListScope>,
}

/// A list scope ready for SQL: the scope plus whether the address index the
/// contact filter reads is complete.
struct ListScopeFilter<'a> {
    scope: &'a ThreadListScope,
    addresses_ready: bool,
}

impl ListScopeFilter<'_> {
    /// ` AND` the thread in `thread_id_column` belongs to the list.
    fn push(&self, query: &mut QueryBuilder<'_, Sqlite>, thread_id_column: &str) {
        query
            .push(" AND EXISTS (SELECT 1 FROM threads WHERE threads.id = ")
            .push(thread_id_column);
        push_thread_scope_filters(
            query,
            self.scope,
            self.addresses_ready,
            FolderLookup::PerThread,
        );
        query.push(")");
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/messages/"))]
pub struct SearchCursor {
    pub rank_cost: f64,
    pub date: String,
    /// Thread id breaks ties after the best matching message is selected.
    pub id: String,
}

#[derive(Debug, Default)]
struct ParsedSearchQuery {
    text_terms: Vec<String>,
    /// `-term` tokens: hits matching any of these are excluded. Unbounded on
    /// their own, so they neither constrain (`has_filters`) nor fill an
    /// otherwise empty query (`is_empty`).
    excluded_terms: Vec<String>,
    from: Option<String>,
    to: Option<String>,
    cc: Option<String>,
    subject: Option<String>,
    in_state: Option<String>,
    tag: Option<String>,
    has_attachment: bool,
    is_unread: bool,
    is_starred: bool,
    before: Option<String>,
    after: Option<String>,
    /// Operators that were recognized but ignored (unknown or invalid value).
    /// Not a constraint — deliberately absent from `has_filters`/`is_empty`.
    warnings: Vec<SearchWarningDto>,
}

impl ParsedSearchQuery {
    /// True when at least one operator constrains the result set, independent
    /// of free-text terms.
    fn has_filters(&self) -> bool {
        self.from.is_some()
            || self.to.is_some()
            || self.cc.is_some()
            || self.subject.is_some()
            || self.in_state.is_some()
            || self.tag.is_some()
            || self.before.is_some()
            || self.after.is_some()
            || self.has_attachment
            || self.is_unread
            || self.is_starred
    }

    /// True when the user typed nothing that constrains the result set — e.g.
    /// only whitespace, or a bare `from:` with no value.
    fn is_empty(&self) -> bool {
        self.text_terms.is_empty() && !self.has_filters()
    }

    /// Flag-only searches can start from the much smaller thread projection.
    /// Keeping this deliberately narrow avoids changing the semantics of
    /// message-level operators such as `from:` or `before:`.
    fn has_only_thread_flags(&self) -> bool {
        (self.has_attachment || self.is_unread || self.is_starred)
            && self.from.is_none()
            && self.to.is_none()
            && self.cc.is_none()
            && self.subject.is_none()
            && self.in_state.is_none()
            && self.tag.is_none()
            && self.before.is_none()
            && self.after.is_none()
            && self.excluded_terms.is_empty()
    }
}

/// Marker bytes wrapped around matched terms in `MessageSearchHitDto::snippet`.
/// Control characters cannot appear in sanitized mail text, so the frontend can
/// split on them without any HTML round-trip (`src/utils/highlight.ts`).
const SEARCH_MARK_START: &str = "\u{1}";
const SEARCH_MARK_END: &str = "\u{2}";

/// Lightweight search hit for the omni panel and the search page.
///
/// Deliberately not a `MessageDto`: search-as-you-type must not pay for body
/// transport, plugin transforms or PGP/S/MIME decryption per row. Opening a hit
/// loads the full message through the regular thread path.
#[derive(Debug, Clone, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/messages/"))]
pub struct MessageSearchHitDto {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    /// FTS `snippet()` output with `SEARCH_MARK_*` around matches, or the
    /// stored snippet (no markers) for operator-only queries.
    pub snippet: String,
    pub from_name: String,
    pub from_address: String,
    pub date: String,
    pub is_read: bool,
    /// Thread workflow state ('inbox' | 'todo' | 'waiting' | 'aside' | 'done' | 'snoozed').
    pub workflow_state: String,
    /// Folder type of the hit message ('sent', 'trash', 'spam', …) or ''.
    pub folder_type: String,
    /// Internal ranking value used to build the continuation cursor.
    #[serde(skip)]
    #[cfg_attr(feature = "ipc-types", ts(skip))]
    pub rank_cost: f64,
    /// The sort date can differ from the displayed draft message date.
    #[serde(skip)]
    #[cfg_attr(feature = "ipc-types", ts(skip))]
    pub cursor_date: String,
}

/// An operator token the parser understood but had to ignore, e.g. `in:` with
/// an unknown mailbox or an unparseable date. Machine-readable on purpose —
/// the user-facing (German) wording lives in the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/messages/"))]
pub struct SearchWarningDto {
    pub operator: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/messages/"))]
pub struct MessageSearchResponseDto {
    pub hits: Vec<MessageSearchHitDto>,
    pub warnings: Vec<SearchWarningDto>,
    pub has_more: bool,
    pub next_cursor: Option<SearchCursor>,
}

#[derive(Debug, Error)]
pub enum MessageSearchError {
    #[error("message search persistence failed: {0}")]
    Persistence(#[from] sqlx::Error),
}

static SEARCH_SLOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Persistence boundary for message search.
pub struct MessageSearchRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> MessageSearchRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn search(
        &self,
        input: &SearchQuery,
    ) -> Result<MessageSearchResponseDto, MessageSearchError> {
        let pool = self.pool;
        let started_at = Instant::now();
        let parsed = parse_search_query(&input.query);
        let empty = |parsed: &ParsedSearchQuery| MessageSearchResponseDto {
            hits: Vec::new(),
            warnings: parsed.warnings.clone(),
            has_more: false,
            next_cursor: None,
        };

        // An empty query would otherwise return the newest messages of the whole
        // mailbox, which reads like a broken search rather than "type something".
        // Warnings still go out: `in:xyz` alone must explain why nothing happens.
        if parsed.is_empty() {
            return Ok(empty(&parsed));
        }

        let fts_match = build_fts_query(&parsed.text_terms, true);
        if fts_match.is_empty() && !parsed.has_filters() {
            // Punctuation-only terms tokenize to nothing; without operators there
            // is no constraint left to search with.
            return Ok(empty(&parsed));
        }

        // A list scope that says nothing a thread could fail is no scope.
        let list_scope = input
            .list_scope
            .as_ref()
            .filter(|scope| !scope.is_unbounded());
        let addresses_ready = match list_scope {
            Some(scope) if scope.needs_addresses() => {
                !data_repairs::is_pending(pool, &data_repairs::MESSAGE_ADDRESSES).await?
            }
            _ => true,
        };
        let list = list_scope.map(|scope| ListScopeFilter {
            scope,
            addresses_ready,
        });

        // One search at a time per process. Search-as-you-type from several
        // windows or panels could otherwise hold most of the read pool with
        // queries whose results are already superseded, stalling thread lists
        // and message loads behind them.
        let _slot = SEARCH_SLOT.lock().await;

        if fts_match.is_empty() && parsed.has_only_thread_flags() {
            let hits = search_thread_flag_hits(pool, input, &parsed, list.as_ref()).await?;
            log_perf(
                "search_message_hits",
                started_at,
                format!("terms=0 rows={} fast=thread_flags", hits.len()),
            );
            return Ok(search_page(hits, parsed.warnings, input));
        }

        if !fts_match.is_empty() && !parsed.has_filters() && parsed.excluded_terms.is_empty() {
            let hits = search_unfiltered_text_hits(pool, input, &fts_match, list.as_ref()).await?;
            log_perf(
                "search_message_hits",
                started_at,
                format!(
                    "terms={} rows={} fast=text_projection",
                    parsed.text_terms.len(),
                    hits.len()
                ),
            );
            return Ok(search_page(hits, parsed.warnings, input));
        }

        // Both branches deduplicate to the best hit per thread: opening a hit lands
        // on the thread anyway, and eight near-identical rows of one conversation
        // would crowd every other result out of the panel.
        let mut query = if fts_match.is_empty() {
            // Operator-only query: no relevance signal, newest first.
            //
            // Only ranking columns are projected before deduplication. `snippet`
            // and the flags are stored after the body columns, so reading them
            // for every candidate walks body overflow pages; display columns are
            // hydrated for the final page instead.
            QueryBuilder::<Sqlite>::new(
                "SELECT hit_rowid as rowid, rank_cost, date as cursor_date FROM (
                SELECT m.rowid as hit_rowid, m.id as id, m.thread_id as thread_id,
                       COALESCE(m.date, '') as date,
                       0.0 as rank_cost,
                       ROW_NUMBER() OVER (
                           PARTITION BY m.thread_id ORDER BY m.date DESC, m.id DESC
                       ) as thread_rank
                FROM messages m
                LEFT JOIN threads t ON t.id = m.thread_id
                LEFT JOIN folders fol ON fol.id = m.folder_id
                JOIN accounts acc ON acc.id = m.account_id AND acc.is_active = 1
                WHERE 1 = 1",
            )
        } else {
            // Free-text query: FTS drives. The rank cost is weighted bm25 (subject
            // over sender/recipients over body; lower is better) plus an age
            // penalty of 0.01 per day (~3.65 per year), so a moderate match from
            // last week outranks a strong subject match from two years ago instead
            // of relevance alone burying everything recent. `snippet(…, -1, …)`
            // picks the best matching column, which keeps ciphertext bodies out of
            // previews when the match was in subject or sender.
            //
            // The common julianday('now') offset is omitted so cursor ranks
            // stay comparable across page requests without changing ordering.
            //
            // The extra nesting is required: the FTS5 bm25 function cannot appear
            // in the same SELECT as a window function, so the inner query computes
            // relevance and the middle layer ranks per thread. Display columns and
            // highlight snippets are intentionally hydrated only for the final
            // page below; computing `snippet()` here would parse every match even
            // though only 50 rows are returned.
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT hit_rowid as rowid, rank_cost, date as cursor_date FROM (
                SELECT fts_hits.*, ROW_NUMBER() OVER (
                    PARTITION BY thread_id ORDER BY rank_cost ASC, date DESC, id DESC
                ) as thread_rank
                FROM (
                    SELECT m.rowid as hit_rowid, m.id as id, m.thread_id as thread_id,
                           COALESCE(m.date, '') as date,
                           bm25(messages_fts, 10.0, 1.0, 5.0, 5.0, 3.0)
                             - COALESCE(julianday(m.date), 0.0) * 0.01 as rank_cost
                    FROM messages_fts
                    JOIN messages m ON m.rowid = messages_fts.rowid
                    LEFT JOIN threads t ON t.id = m.thread_id
                    LEFT JOIN folders fol ON fol.id = m.folder_id
                    JOIN accounts acc ON acc.id = m.account_id AND acc.is_active = 1
                    WHERE messages_fts MATCH ",
            );
            query.push_bind(fts_match.clone());
            query
        };

        // Until the backfill is through, the index misses older messages.
        let cc_index_ready =
            parsed.cc.is_none() || !data_repairs::is_pending(pool, &data_repairs::CC_INDEX).await?;
        push_search_filters(
            &mut query,
            &parsed,
            input.account_id.as_deref(),
            cc_index_ready,
        );
        if let Some(list) = &list {
            list.push(&mut query, "m.thread_id");
        }

        if fts_match.is_empty() {
            query.push(") WHERE thread_rank = 1");
            push_search_cursor(&mut query, input, false, "date", "thread_id");
            query.push(" ORDER BY date DESC, thread_id DESC LIMIT ");
        } else {
            query.push(") as fts_hits ) WHERE thread_rank = 1");
            push_search_cursor(&mut query, input, true, "date", "thread_id");
            query.push(" ORDER BY rank_cost ASC, date DESC, thread_id DESC LIMIT ");
        }
        push_search_page(&mut query, input);

        let ranked = query
            .build_query_as::<RankedSearchRow>()
            .fetch_all(pool)
            .await?;
        let hits = hydrate_search_hits(
            pool,
            (!fts_match.is_empty()).then_some(fts_match.as_str()),
            ranked,
        )
        .await?;

        log_perf(
            "search_message_hits",
            started_at,
            format!("terms={} rows={}", parsed.text_terms.len(), hits.len()),
        );
        Ok(search_page(hits, parsed.warnings, input))
    }
}

fn push_search_cursor(
    query: &mut QueryBuilder<'_, Sqlite>,
    input: &SearchQuery,
    ranked: bool,
    date_column: &str,
    id_column: &str,
) {
    let Some(cursor) = &input.cursor else { return };
    query.push(" AND (");
    if ranked {
        query.push("rank_cost > ").push_bind(cursor.rank_cost);
        query.push(" OR (rank_cost = ").push_bind(cursor.rank_cost);
        query.push(" AND (");
    }
    query
        .push(date_column)
        .push(" < ")
        .push_bind(cursor.date.clone());
    query
        .push(" OR (")
        .push(date_column)
        .push(" = ")
        .push_bind(cursor.date.clone());
    query
        .push(" AND ")
        .push(id_column)
        .push(" < ")
        .push_bind(cursor.id.clone())
        .push(")");
    if ranked {
        query.push("))");
    }
    query.push(")");
}

fn push_search_page(query: &mut QueryBuilder<'_, Sqlite>, input: &SearchQuery) {
    query.push_bind(input.limit.unwrap_or(50).clamp(1, 500) + 1);
}

fn search_page(
    mut hits: Vec<MessageSearchHitDto>,
    warnings: Vec<SearchWarningDto>,
    input: &SearchQuery,
) -> MessageSearchResponseDto {
    let limit = input.limit.unwrap_or(50).clamp(1, 500) as usize;
    let has_more = hits.len() > limit;
    hits.truncate(limit);
    MessageSearchResponseDto {
        next_cursor: if has_more {
            hits.last().map(|hit| SearchCursor {
                rank_cost: hit.rank_cost,
                date: hit.cursor_date.clone(),
                id: hit.thread_id.clone(),
            })
        } else {
            None
        },
        hits,
        warnings,
        has_more,
    }
}

/// A ranked, deduplicated search hit before its display columns are loaded.
#[derive(FromRow)]
struct RankedSearchRow {
    /// `messages.rowid`, which is also the `messages_fts` rowid.
    rowid: i64,
    rank_cost: f64,
    cursor_date: String,
}

/// Upper bound on the matches an unfiltered text search ranks.
///
/// bm25 and the per-thread window run for every match before `LIMIT`, so a
/// broad term (`m*` while typing, a word in every signature) cost seconds on a
/// large mailbox. Above this many matches only the newest ones are ranked;
/// the age penalty in `rank_cost` buries older matches below them in all but
/// extreme cases. Below it, the candidate set is the whole match set and
/// ranking is exact.
const UNFILTERED_SEARCH_CANDIDATES: i64 = 5_000;

/// Ranks ordinary free-text searches against a compact message projection.
///
/// The `messages` table stores full bodies and is several gigabytes for a
/// large mailbox. FTS yields rowids in relevance order, but joining every
/// match back to that table caused thousands of random body-table page reads
/// before returning 50 rows. The projection contains only ranking columns;
/// full display data is loaded after deduplication and limiting.
async fn search_unfiltered_text_hits(
    pool: &SqlitePool,
    input: &SearchQuery,
    fts_match: &str,
    list: Option<&ListScopeFilter<'_>>,
) -> Result<Vec<MessageSearchHitDto>, MessageSearchError> {
    let ranked =
        unfiltered_text_ranking_query(input, fts_match, UNFILTERED_SEARCH_CANDIDATES, list)
            .build_query_as::<RankedSearchRow>()
            .fetch_all(pool)
            .await?;
    hydrate_search_hits(pool, Some(fts_match), ranked).await
}

fn unfiltered_text_ranking_query(
    input: &SearchQuery,
    fts_match: &str,
    candidate_cap: i64,
    list: Option<&ListScopeFilter<'_>>,
) -> QueryBuilder<'static, Sqlite> {
    let account_id = input
        .account_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT hit_rowid as rowid, rank_cost, date as cursor_date FROM (
            SELECT ranked.*, ROW_NUMBER() OVER (
                PARTITION BY thread_id ORDER BY rank_cost ASC, date DESC, id DESC
            ) as thread_rank
            FROM (
                SELECT sp.message_rowid as hit_rowid,
                       sp.message_id as id,
                       sp.thread_id as thread_id,
                       sp.date as date,
                       bm25(messages_fts, 10.0, 1.0, 5.0, 5.0, 3.0)
                         - COALESCE(julianday(sp.date), 0.0) * 0.01 as rank_cost
                FROM messages_fts
                JOIN message_search_projection sp ON sp.message_rowid = messages_fts.rowid
                JOIN accounts acc ON acc.id = sp.account_id AND acc.is_active = 1
                WHERE messages_fts MATCH ",
    );
    query.push_bind(fts_match.to_string());
    if let Some(account_id) = &account_id {
        query.push(" AND sp.account_id = ");
        query.push_bind(account_id.clone());
    } else {
        query
            .push(" AND ")
            .push(crate::db::account_visibility::visible_account_sql(
                "sp.account_id",
            ));
    }
    if let Some(list) = list {
        list.push(&mut query, "sp.thread_id");
    }
    // The date of the `candidate_cap`-th newest match, or '' (every match) when
    // there are fewer. Finding it reads only doclist rowids and the compact
    // projection; bm25 then runs for the candidates alone.
    query.push(
        " AND sp.date >= COALESCE((
            SELECT cap.date
            FROM messages_fts cap_fts
            JOIN message_search_projection cap ON cap.message_rowid = cap_fts.rowid
            JOIN accounts cap_acc ON cap_acc.id = cap.account_id AND cap_acc.is_active = 1
            WHERE cap_fts.messages_fts MATCH ",
    );
    query.push_bind(fts_match.to_string());
    if let Some(account_id) = account_id {
        query.push(" AND cap.account_id = ");
        query.push_bind(account_id);
    } else {
        query
            .push(" AND ")
            .push(crate::db::account_visibility::visible_account_sql(
                "cap.account_id",
            ));
    }
    // The cap counts the list's matches, or a scoped search would rank
    // fewer candidates than an unscoped one.
    if let Some(list) = list {
        list.push(&mut query, "cap.thread_id");
    }
    query.push(" ORDER BY cap.date DESC LIMIT 1 OFFSET ");
    query.push_bind(candidate_cap.max(1) - 1);
    query.push("), '')");
    query.push(") as ranked ) WHERE thread_rank = 1");
    push_search_cursor(&mut query, input, true, "date", "thread_id");
    query.push(" ORDER BY rank_cost ASC, date DESC, thread_id DESC LIMIT ");
    push_search_page(&mut query, input);
    query
}

#[derive(FromRow)]
struct HydratedSearchRow {
    rowid: i64,
    id: String,
    thread_id: String,
    subject: String,
    snippet: String,
    from_name: String,
    from_address: String,
    date: String,
    is_read: bool,
    workflow_state: String,
    folder_type: String,
}

/// Loads display columns, and FTS highlights for text queries, for one ranked
/// page, keeping the ranking order.
///
/// Rows are addressed by rowid. The FTS variant must still MATCH, since
/// `snippet()` needs the match cursor, but `+rowid IN (…)` keeps the rowids
/// out of the FTS index constraint: as an index constraint FTS5 re-runs the
/// query once per rowid (painful for prefix terms), while as a filter the
/// doclist is walked once, bounded by the page's rowid range, and only page
/// rows join `messages`. Filtering on `m.id` instead joined every match.
/// A message deleted between the two reads drops out of the page.
async fn hydrate_search_hits(
    pool: &SqlitePool,
    fts_match: Option<&str>,
    ranked: Vec<RankedSearchRow>,
) -> Result<Vec<MessageSearchHitDto>, MessageSearchError> {
    if ranked.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT m.rowid as rowid, m.id as id, m.thread_id as thread_id,
                COALESCE(m.subject, '') as subject, ",
    );
    if fts_match.is_some() {
        query.push("snippet(messages_fts, -1, ");
        query.push_bind(SEARCH_MARK_START);
        query.push(", ");
        query.push_bind(SEARCH_MARK_END);
        query.push(", ' … ', 14)");
    } else {
        query.push("COALESCE(m.snippet, '')");
    }
    query.push(
        " as snippet,
                COALESCE(m.from_name, '') as from_name,
                COALESCE(m.from_address, '') as from_address,
                COALESCE(m.date, '') as date,
                COALESCE(m.is_read, 0) as is_read,
                COALESCE(t.workflow_state, 'inbox') as workflow_state,
                COALESCE(fol.folder_type, '') as folder_type",
    );
    if let Some(fts_match) = fts_match {
        let low = ranked.iter().map(|row| row.rowid).min().unwrap_or_default();
        let high = ranked.iter().map(|row| row.rowid).max().unwrap_or_default();
        query.push(
            " FROM messages_fts
              JOIN messages m ON m.rowid = messages_fts.rowid
              LEFT JOIN threads t ON t.id = m.thread_id
              LEFT JOIN folders fol ON fol.id = m.folder_id
              WHERE messages_fts MATCH ",
        );
        query.push_bind(fts_match.to_string());
        query.push(" AND messages_fts.rowid BETWEEN ");
        query.push_bind(low);
        query.push(" AND ");
        query.push_bind(high);
        query.push(" AND +messages_fts.rowid IN (");
    } else {
        query.push(
            " FROM messages m
              LEFT JOIN threads t ON t.id = m.thread_id
              LEFT JOIN folders fol ON fol.id = m.folder_id
              WHERE m.rowid IN (",
        );
    }
    let mut rowids = query.separated(", ");
    for row in &ranked {
        rowids.push_bind(row.rowid);
    }
    rowids.push_unseparated(")");

    let mut hydrated = query
        .build_query_as::<HydratedSearchRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.rowid, row))
        .collect::<HashMap<_, _>>();

    Ok(ranked
        .into_iter()
        .filter_map(|ranked| {
            hydrated
                .remove(&ranked.rowid)
                .map(|row| MessageSearchHitDto {
                    id: row.id,
                    thread_id: row.thread_id,
                    subject: repair_likely_utf8_mojibake(&decode_mime_encoded_words(&row.subject)),
                    snippet: row.snippet,
                    from_name: repair_likely_utf8_mojibake(&decode_mime_encoded_words(
                        &row.from_name,
                    )),
                    from_address: row.from_address,
                    date: row.date,
                    is_read: row.is_read,
                    workflow_state: row.workflow_state,
                    folder_type: row.folder_type,
                    rank_cost: ranked.rank_cost,
                    cursor_date: ranked.cursor_date,
                })
        })
        .collect())
}

/// Fast path for `is:unread`, `is:starred` and `has:attachment`.
///
/// The generic operator query ranks every matching message with a window
/// function before it can keep one row per thread. On a large mailbox that
/// means reading and sorting tens of thousands of rows for a 50-row result.
/// These three flags already exist in the thread projection, so start there
/// and fetch only one representative message per matching thread.
async fn search_thread_flag_hits(
    pool: &SqlitePool,
    input: &SearchQuery,
    parsed: &ParsedSearchQuery,
    list: Option<&ListScopeFilter<'_>>,
) -> Result<Vec<MessageSearchHitDto>, MessageSearchError> {
    let date_column = if parsed.is_unread {
        "m.date"
    } else {
        "t.last_message_date"
    };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT m.id as id, m.thread_id as thread_id,
                COALESCE(m.subject, '') as subject,
                COALESCE(m.snippet, '') as snippet,
                COALESCE(m.from_name, '') as from_name,
                COALESCE(m.from_address, '') as from_address,
                COALESCE(m.date, '') as date,
                COALESCE({date_column}, '') as cursor_date,
                COALESCE(m.is_read, 0) as is_read,
                COALESCE(t.workflow_state, 'inbox') as workflow_state,
                COALESCE(fol.folder_type, '') as folder_type,
                0.0 as rank_cost
         FROM threads t
         JOIN accounts acc ON acc.id = t.account_id AND acc.is_active = 1
         JOIN messages m ON m.id = (
             SELECT candidate.id
             FROM messages candidate
             WHERE candidate.thread_id = t.id"
    ));

    // Preserve the generic query's result row: an unread search points at the
    // newest unread message, while the thread-level flags point at the newest
    // message in that matching conversation.
    if parsed.is_unread {
        query.push(" AND candidate.is_read = 0");
    }
    query.push(
        " ORDER BY candidate.date DESC, candidate.id DESC
          LIMIT 1
         )
         LEFT JOIN folders fol ON fol.id = m.folder_id
         WHERE 1 = 1",
    );

    if let Some(account_id) = input.account_id.as_deref().filter(|id| !id.is_empty()) {
        query.push(" AND t.account_id = ");
        query.push_bind(account_id.to_string());
    } else {
        query
            .push(" AND ")
            .push(crate::db::account_visibility::visible_account_sql(
                "t.account_id",
            ));
    }
    if parsed.has_attachment {
        query.push(" AND t.has_attachments = 1");
    }
    if parsed.is_unread {
        query.push(" AND t.unread_count > 0");
    }
    if parsed.is_starred {
        query.push(" AND t.is_starred = 1");
    }
    if let Some(list) = list {
        list.push(&mut query, "t.id");
    }

    push_search_cursor(&mut query, input, false, date_column, "t.id");
    query
        .push(" ORDER BY ")
        .push(date_column)
        .push(" DESC, t.id DESC");
    query.push(" LIMIT ");
    push_search_page(&mut query, input);

    let rows = query
        .build_query_as::<MessageSearchHitDto>()
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|mut hit| {
            hit.subject = repair_likely_utf8_mojibake(&decode_mime_encoded_words(&hit.subject));
            hit.from_name = repair_likely_utf8_mojibake(&decode_mime_encoded_words(&hit.from_name));
            hit
        })
        .collect())
}

/// Applies every operator constraint from `parse_search_query` plus the
/// account scope. Shared by both search branches so `from:`/`in:`/`tag:`/…
/// behave identically with and without free text.
fn push_search_filters(
    query: &mut QueryBuilder<'_, Sqlite>,
    parsed: &ParsedSearchQuery,
    account_id: Option<&str>,
    cc_index_ready: bool,
) {
    if let Some(account_id) = account_id.filter(|id| !id.is_empty()) {
        query.push(" AND m.account_id = ");
        query.push_bind(account_id.to_string());
    } else {
        query
            .push(" AND ")
            .push(crate::db::account_visibility::visible_account_sql(
                "m.account_id",
            ));
    }

    if let Some(from) = &parsed.from {
        let looks_like_full_address = from.contains('@')
            && from
                .rsplit('@')
                .next()
                .is_some_and(|domain| domain.contains('.'));
        if looks_like_full_address {
            // A complete address (the contact-search hot path) matches exactly
            // via idx_messages_from_address_nocase instead of LIKE-scanning
            // the whole messages table.
            query.push(" AND m.from_address = ");
            query.push_bind(from.to_string());
            query.push(" COLLATE NOCASE");
        } else {
            // Address fragment OR display name: `from:"Mara Vogel"` must work
            // even though the display name never appears in the address.
            query.push(" AND (m.from_address LIKE ");
            query.push_bind(format!("%{from}%"));
            query.push(" OR m.from_name LIKE ");
            query.push_bind(format!("%{from}%"));
            query.push(")");
        }
    }

    if let Some(to) = &parsed.to {
        push_recipient_filter(query, "messages_fts", "to_addresses", to);
    }

    if let Some(cc) = &parsed.cc {
        if cc_index_ready {
            push_recipient_filter(query, "message_cc_fts", "cc_addresses", cc);
        } else {
            query.push(" AND m.cc_addresses LIKE ");
            query.push_bind(format!("%{cc}%"));
        }
    }

    if let Some(subject) = &parsed.subject {
        query.push(" AND m.subject LIKE ");
        query.push_bind(format!("%{subject}%"));
    }

    if let Some(in_state) = &parsed.in_state {
        match in_state.as_str() {
            "inbox" | "todo" | "waiting" | "aside" | "done" | "snoozed" => {
                query.push(" AND t.workflow_state = ");
                query.push_bind(in_state.clone());
            }
            "sent" | "drafts" | "trash" | "archive" | "spam" => {
                query.push(
                    " AND EXISTS (
                        SELECT 1
                        FROM folders f
                        WHERE f.id = m.folder_id
                          AND f.folder_type = ",
                );
                query.push_bind(in_state.clone());
                query.push(")");
            }
            _ => {}
        }
    }

    if let Some(tag) = &parsed.tag {
        query.push(
            " AND EXISTS (
                SELECT 1
                FROM thread_tags tt
                JOIN tags tg ON tg.id = tt.tag_id
                WHERE tt.thread_id = m.thread_id
                  AND tg.name LIKE ",
        );
        query.push_bind(format!("%{tag}%"));
        query.push(")");
    }

    if parsed.has_attachment {
        query.push(" AND (m.has_attachments = 1 OR COALESCE(t.has_attachments, 0) = 1)");
    }

    if parsed.is_unread {
        query.push(" AND m.is_read = 0");
    }

    if parsed.is_starred {
        query.push(" AND (m.is_starred = 1 OR COALESCE(t.is_starred, 0) = 1)");
    }

    if let Some(before) = &parsed.before {
        query.push(" AND m.date < ");
        query.push_bind(before.clone());
    }

    if let Some(after) = &parsed.after {
        query.push(" AND m.date >= ");
        query.push_bind(after.clone());
    }

    let exclusion = build_fts_exclusion(&parsed.excluded_terms);
    if !exclusion.is_empty() {
        // Subquery instead of FTS NOT syntax: it works identically for the
        // ranked and the operator-only branch.
        query.push(" AND m.rowid NOT IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ");
        query.push_bind(exclusion);
        query.push(")");
    }
}

/// Splits a raw query into tokens, keeping double-quoted spans together:
/// `from:"Mara Vogel" "genauer wortlaut"` yields two tokens. Quotes stay in
/// the token; the consumers strip them (`unquote` for operator values, the FTS
/// builders for text terms — where the kept quotes turn into phrase queries).
fn tokenize_query(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push('"');
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Strips the phrase quotes from an operator value: `"Mara Vogel"` → `Mara Vogel`.
fn unquote(value: &str) -> String {
    value.replace('"', "")
}

fn parse_search_query(raw: &str) -> ParsedSearchQuery {
    let mut parsed = ParsedSearchQuery::default();
    for token in tokenize_query(raw) {
        let token = token.as_str();
        // Negation first so `-from:x` and friends never fall through to the
        // operator arms below.
        if let Some(value) = token.strip_prefix('-') {
            if !value.is_empty() {
                parsed.excluded_terms.push(value.to_string());
            }
            continue;
        }

        if let Some(value) = token.strip_prefix("from:") {
            let value = unquote(value);
            parsed.from = (!value.is_empty()).then_some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("to:") {
            let value = unquote(value);
            parsed.to = (!value.is_empty()).then_some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("cc:") {
            let value = unquote(value);
            parsed.cc = (!value.is_empty()).then_some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("subject:") {
            let value = unquote(value);
            parsed.subject = (!value.is_empty()).then_some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("in:") {
            if value.is_empty() {
                continue;
            }
            let value = value.to_lowercase();
            match value.as_str() {
                // Workflow states and folder types; must match push_search_filters.
                "inbox" | "todo" | "waiting" | "aside" | "done" | "snoozed" | "sent" | "drafts"
                | "trash" | "archive" | "spam" => parsed.in_state = Some(value),
                _ => parsed.warnings.push(SearchWarningDto {
                    operator: "in".into(),
                    value,
                }),
            }
            continue;
        }

        if let Some(value) = token.strip_prefix("tag:") {
            let value = unquote(value);
            parsed.tag = (!value.is_empty()).then_some(value.to_lowercase());
            continue;
        }

        if let Some(value) = token.strip_prefix("has:") {
            if value.eq_ignore_ascii_case("attachment") {
                parsed.has_attachment = true;
            } else if !value.is_empty() {
                parsed.warnings.push(SearchWarningDto {
                    operator: "has".into(),
                    value: value.to_lowercase(),
                });
            }
            continue;
        }

        if let Some(value) = token.strip_prefix("is:") {
            match value.to_ascii_lowercase().as_str() {
                "unread" => parsed.is_unread = true,
                "starred" | "flagged" => parsed.is_starred = true,
                "" => {}
                other => parsed.warnings.push(SearchWarningDto {
                    operator: "is".into(),
                    value: other.into(),
                }),
            }
            continue;
        }

        if let Some(value) = token.strip_prefix("before:") {
            parsed.before = parse_search_date(value);
            if parsed.before.is_none() && !value.trim().is_empty() {
                parsed.warnings.push(SearchWarningDto {
                    operator: "before".into(),
                    value: value.into(),
                });
            }
            continue;
        }

        if let Some(value) = token.strip_prefix("after:") {
            parsed.after = parse_search_date(value);
            if parsed.after.is_none() && !value.trim().is_empty() {
                parsed.warnings.push(SearchWarningDto {
                    operator: "after".into(),
                    value: value.into(),
                });
            }
            continue;
        }

        let clean = token.trim();
        if !clean.is_empty() {
            parsed.text_terms.push(clean.to_string());
        }
    }
    parsed
}

fn parse_search_date(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return midnight_utc(date);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&chrono::Utc).to_rfc3339());
    }
    parse_relative_date(value)
}

/// `heute`, `gestern` and `<n><d|w|m|y>` (e.g. `7d`, `2w`), counted back from
/// today. Months and years are approximated as 30/365 days — for a search
/// window that is precise enough.
fn parse_relative_date(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let today = chrono::Utc::now().date_naive();

    let date = match lower.as_str() {
        "heute" | "today" => today,
        "gestern" | "yesterday" => today - chrono::Duration::days(1),
        _ => {
            let unit = lower.chars().last()?;
            let amount: i64 = lower.get(..lower.len() - 1)?.parse().ok()?;
            let days = match unit {
                'd' => amount,
                'w' => amount * 7,
                'm' => amount * 30,
                'y' => amount * 365,
                _ => return None,
            };
            today - chrono::Duration::days(days)
        }
    };
    midnight_utc(date)
}

fn midnight_utc(date: chrono::NaiveDate) -> Option<String> {
    let dt = date.and_hms_opt(0, 0, 0)?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc).to_rfc3339())
}

/// Turns free-text terms into a safe FTS5 MATCH expression.
///
/// Every term is stripped of `"` and wrapped in a phrase quote, so FTS syntax
/// (`OR`, `NEAR`, column filters) cannot be injected. Terms without a single
/// alphanumeric character are dropped entirely — they tokenize to an empty
/// phrase and would raise a MATCH syntax error. With `prefix_last`, the final
/// term becomes a phrase-prefix (`"term"*`; the star sits outside the quotes)
/// so search-as-you-type matches "Rechnung" while the user is still at "rechn".
/// Strips quote characters and drops terms that tokenize to nothing (they
/// would raise an FTS MATCH syntax error).
fn clean_fts_terms(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .map(|term| term.replace('"', ""))
        .filter(|term| term.chars().any(char::is_alphanumeric))
        .collect()
}

/// MATCH expression that is true when ANY excluded term matches. Used inside a
/// `NOT IN` subquery, so OR is the correct join; no prefix star — excluding
/// every completion of a half-typed word would be surprising.
fn build_fts_exclusion(terms: &[String]) -> String {
    clean_fts_terms(terms)
        .iter()
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Whole address tokens: `to:ann` finds `ann@…` and `ann.meier@…`, not `joann@…`.
fn push_recipient_filter(
    query: &mut QueryBuilder<'_, Sqlite>,
    table: &'static str,
    column: &'static str,
    term: &str,
) {
    let cleaned = term.replace('"', "");
    if !cleaned.chars().any(char::is_alphanumeric) {
        query.push(" AND 0");
        return;
    }
    query.push(format!(
        " AND m.rowid IN (SELECT rowid FROM {table} WHERE {table} MATCH "
    ));
    query.push_bind(format!("{column} : \"{cleaned}\""));
    query.push(")");
}

fn build_fts_query(terms: &[String], prefix_last: bool) -> String {
    let cleaned = clean_fts_terms(terms);

    let last_index = cleaned.len().saturating_sub(1);
    cleaned
        .iter()
        .enumerate()
        .map(|(index, term)| {
            if prefix_last
                && index == last_index
                && term
                    .rsplit(|c: char| !c.is_alphanumeric())
                    .find(|token| !token.is_empty())
                    .is_some_and(|token| token.chars().count() >= 2)
            {
                format!("\"{term}\"*")
            } else {
                format!("\"{term}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn log_perf(operation: &str, started_at: Instant, detail: String) {
    let enabled = std::env::var("SATCHEL_PERF_LOG")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if !enabled {
        return;
    }

    tracing::info!(
        target: "perf",
        "{} {}ms {}",
        operation,
        started_at.elapsed().as_millis(),
        detail
    );
}

#[cfg(test)]
mod tests {
    use super::{build_fts_query, parse_search_date, parse_search_query};

    use super::{
        unfiltered_text_ranking_query, RankedSearchRow, SearchCursor, SearchQuery,
        UNFILTERED_SEARCH_CANDIDATES,
    };
    use sqlx::{sqlite::SqlitePoolOptions, QueryBuilder, Sqlite, SqlitePool};

    /// The unfiltered ranking query as it was before the candidate cap.
    fn uncapped_reference(input: &SearchQuery, fts_match: &str) -> QueryBuilder<'static, Sqlite> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT hit_rowid as rowid, rank_cost, date as cursor_date FROM (
                SELECT ranked.*, ROW_NUMBER() OVER (
                    PARTITION BY thread_id ORDER BY rank_cost ASC, date DESC, id DESC
                ) as thread_rank
                FROM (
                    SELECT sp.message_rowid as hit_rowid, sp.message_id as id,
                           sp.thread_id as thread_id, sp.date as date,
                           bm25(messages_fts, 10.0, 1.0, 5.0, 5.0, 3.0)
                             - COALESCE(julianday(sp.date), 0.0) * 0.01 as rank_cost
                    FROM messages_fts
                    JOIN message_search_projection sp ON sp.message_rowid = messages_fts.rowid
                    JOIN accounts acc ON acc.id = sp.account_id AND acc.is_active = 1
                    WHERE messages_fts MATCH ",
        );
        query.push_bind(fts_match.to_string());
        if let Some(account_id) = input.account_id.as_deref().filter(|id| !id.is_empty()) {
            query.push(" AND sp.account_id = ");
            query.push_bind(account_id.to_string());
        }
        query.push(") as ranked ) WHERE thread_rank = 1");
        super::push_search_cursor(&mut query, input, true, "date", "thread_id");
        query.push(" ORDER BY rank_cost ASC, date DESC, thread_id DESC LIMIT ");
        super::push_search_page(&mut query, input);
        query
    }

    async fn ranked(
        mut query: QueryBuilder<'static, Sqlite>,
        pool: &SqlitePool,
    ) -> Vec<(i64, f64)> {
        query
            .build_query_as::<RankedSearchRow>()
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.rowid, row.rank_cost))
            .collect()
    }

    async fn seeded_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::raw_sql(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES ('a0', 'A', 'a0@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01'),
                    ('a1', 'B', 'a1@x.test', 'imap', '{}', '', 1, '2026-01-01', '2026-01-01');
             INSERT INTO folders(id, account_id, name, full_path, folder_type, created_at, updated_at)
             SELECT 'inbox-' || id, id, 'Inbox', 'INBOX', 'inbox', '2026-01-01', '2026-01-01' FROM accounts;
             -- Rowids run against dates (a backfill inserts older mail later),
             -- several messages share a thread and some strong subject matches
             -- are old.
             WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x + 1 FROM n WHERE x < 119)
             INSERT INTO messages(id, account_id, folder_id, thread_id, from_address, to_addresses,
                                  subject, body_text, date, created_at, updated_at)
             SELECT printf('m%03d', x), 'a' || (x % 2), 'inbox-a' || (x % 2), printf('t%03d', x / 3),
                    'sender' || x || '@x.test', '[]',
                    CASE WHEN x % 7 = 0 THEN 'Budget review' ELSE 'Weekly notes' END,
                    CASE WHEN x % 4 = 0 THEN 'budget budget figures' ELSE 'budget figures' END,
                    printf('2026-%02d-%02dT10:00:00Z', 12 - (x / 28) % 12, 1 + x % 28),
                    '2026-01-01', '2026-01-01'
             FROM n;",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn input(account_id: Option<&str>, limit: i64) -> SearchQuery {
        SearchQuery {
            query: String::new(),
            account_id: account_id.map(str::to_string),
            limit: Some(limit),
            cursor: None,
            list_scope: None,
        }
    }

    #[tokio::test]
    async fn unfiltered_ranking_matches_the_uncapped_query_below_the_cap() {
        let pool = seeded_pool().await;
        for fts_match in [
            "\"budget\"",
            "\"bud\"*",
            "\"budget\" AND \"review\"",
            "\"missing\"",
        ] {
            for account_id in [None, Some("a1")] {
                let mut page = input(account_id, 7);
                loop {
                    let expected = ranked(uncapped_reference(&page, fts_match), &pool).await;
                    let actual = ranked(
                        unfiltered_text_ranking_query(
                            &page,
                            fts_match,
                            UNFILTERED_SEARCH_CANDIDATES,
                            None,
                        ),
                        &pool,
                    )
                    .await;
                    assert_eq!(actual, expected, "{fts_match} {account_id:?}");
                    if expected.len() <= 7 {
                        break;
                    }
                    let last = expected[6];
                    let cursor_row: (String, String) =
                        sqlx::query_as("SELECT date, thread_id FROM messages WHERE rowid = ?")
                            .bind(last.0)
                            .fetch_one(&pool)
                            .await
                            .unwrap();
                    page.cursor = Some(SearchCursor {
                        rank_cost: last.1,
                        date: cursor_row.0,
                        id: cursor_row.1,
                    });
                }
            }
        }
    }

    #[tokio::test]
    async fn unfiltered_ranking_ranks_only_the_newest_candidates_of_a_broad_match() {
        let pool = seeded_pool().await;
        let cap = 20;
        let newest: Vec<i64> =
            sqlx::query_scalar("SELECT rowid FROM messages ORDER BY date DESC LIMIT ?")
                .bind(cap)
                .fetch_all(&pool)
                .await
                .unwrap();
        let cutoff: String = sqlx::query_scalar(
            "SELECT MIN(date) FROM messages WHERE rowid IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(&newest).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        let hits = ranked(
            unfiltered_text_ranking_query(&input(None, 500), "\"budget\"", cap, None),
            &pool,
        )
        .await;
        assert!(!hits.is_empty());
        for (rowid, _) in &hits {
            let date: String = sqlx::query_scalar("SELECT date FROM messages WHERE rowid = ?")
                .bind(rowid)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(
                date >= cutoff,
                "{rowid} {date} is older than the candidates"
            );
        }
        // Every thread among the newest candidates is still represented.
        let threads: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT thread_id) FROM messages WHERE date >= ?")
                .bind(&cutoff)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(hits.len() as i64, threads);
    }

    #[test]
    fn one_character_terms_do_not_expand_prefixes() {
        assert_eq!(build_fts_query(&["m".into()], true), "\"m\"");
        assert_eq!(build_fts_query(&["ü".into()], true), "\"ü\"");
        assert_eq!(build_fts_query(&["ma".into()], true), "\"ma\"*");
        assert_eq!(
            build_fts_query(&["memo".into(), "m".into()], true),
            "\"memo\" AND \"m\""
        );
    }

    #[test]
    fn keeps_quoted_phrases_together() {
        let parsed =
            parse_search_query(r#"from:"Mara Vogel" "genauer wortlaut" -"zoom meeting" rest"#);
        assert_eq!(parsed.from.as_deref(), Some("mara vogel"));
        assert_eq!(parsed.text_terms, vec![r#""genauer wortlaut""#, "rest"]);
        assert_eq!(parsed.excluded_terms, vec![r#""zoom meeting""#]);

        // The kept quotes become one FTS phrase, not two AND-terms.
        let fts = build_fts_query(&parsed.text_terms, true);
        assert_eq!(fts, r#""genauer wortlaut" AND "rest"*"#);
    }

    #[test]
    fn parses_relative_dates() {
        for raw in [
            "7d",
            "2w",
            "1m",
            "1y",
            "heute",
            "gestern",
            "today",
            "yesterday",
        ] {
            assert!(
                parse_search_date(raw).is_some(),
                "expected {raw:?} to parse"
            );
        }
        // Relative dates count backwards from today.
        let week_ago = parse_search_date("7d").unwrap();
        let yesterday = parse_search_date("gestern").unwrap();
        let today = parse_search_date("heute").unwrap();
        assert!(week_ago < yesterday && yesterday < today);

        for raw in ["morgen", "d7", "x", "7 tage"] {
            assert!(parse_search_date(raw).is_none(), "expected {raw:?} to fail");
        }
    }

    #[test]
    fn extracts_negated_terms() {
        let parsed = parse_search_query("mike -code.example -zoom");
        assert_eq!(parsed.text_terms, vec!["mike"]);
        assert_eq!(parsed.excluded_terms, vec!["code.example", "zoom"]);
        // A lone dash is noise, not an exclusion.
        assert!(parse_search_query("-").excluded_terms.is_empty());
    }

    #[test]
    fn treats_blank_and_valueless_queries_as_empty() {
        // These must short-circuit: otherwise search returns the newest mail of
        // the entire mailbox and looks broken. `-zoom` alone is unbounded and
        // counts as empty too.
        for raw in ["", "   ", "\t\n", "from:", "subject:", "-zoom"] {
            assert!(
                parse_search_query(raw).is_empty(),
                "expected {raw:?} to parse as an empty query"
            );
        }
    }

    #[test]
    fn recognises_constraining_queries_as_non_empty() {
        for raw in [
            "rechnung",
            "from:a@b.test",
            "has:attachment",
            "is:unread",
            "is:starred",
            "tag:wichtig",
            "in:sent",
            "after:2026-01-01",
        ] {
            assert!(
                !parse_search_query(raw).is_empty(),
                "expected {raw:?} to parse as a constraining query"
            );
        }
    }

    #[test]
    fn keeps_free_text_separate_from_operators() {
        let parsed = parse_search_query("from:a@b.test rechnung märz has:attachment");
        assert_eq!(parsed.from.as_deref(), Some("a@b.test"));
        assert!(parsed.has_attachment);
        assert_eq!(parsed.text_terms, vec!["rechnung", "märz"]);
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn warns_on_operator_values_it_has_to_ignore() {
        // `before:gestern` would be valid (relative date) — `irgendwann` is not.
        let parsed = parse_search_query("in:xyz is:foo has:pdf before:irgendwann after:2026-01-01");
        let warned: Vec<(&str, &str)> = parsed
            .warnings
            .iter()
            .map(|warning| (warning.operator.as_str(), warning.value.as_str()))
            .collect();
        assert_eq!(
            warned,
            vec![
                ("in", "xyz"),
                ("is", "foo"),
                ("has", "pdf"),
                ("before", "irgendwann")
            ],
        );
        // The ignored operators set no constraints; the valid after: does.
        assert!(parsed.in_state.is_none());
        assert!(!parsed.is_unread && !parsed.is_starred && !parsed.has_attachment);
        assert!(parsed.before.is_none());
        assert!(parsed.after.is_some());
    }

    #[test]
    fn a_query_of_only_ignored_operators_counts_as_empty() {
        // `in:xyz` used to slip through as a non-empty query with no effective
        // filter and returned the newest mail of the whole mailbox.
        let parsed = parse_search_query("in:xyz is:foo");
        assert!(parsed.is_empty());
        assert_eq!(parsed.warnings.len(), 2);
    }

    #[test]
    fn quotes_fts_terms_so_operators_cannot_be_injected() {
        // A bare `"` or an FTS keyword must not turn into query syntax.
        let parsed = parse_search_query(r#"rechnung" OR body:x"#);
        let fts = build_fts_query(&parsed.text_terms, false);
        assert!(fts.starts_with('"'));
        assert!(!fts.contains(r#"""#.repeat(2).as_str()));
        for term in &parsed.text_terms {
            assert!(!term.is_empty());
        }
    }

    #[test]
    fn joins_multiple_terms_with_and() {
        let fts = build_fts_query(&["rechnung".to_string(), "märz".to_string()], false);
        assert_eq!(fts, "\"rechnung\" AND \"märz\"");
    }

    #[test]
    fn prefix_star_stays_outside_quoted_phrase() {
        // Only the last term becomes a phrase-prefix; the star must sit outside
        // the quotes (FTS5 syntax) so it cannot be smuggled in as term content.
        let fts = build_fts_query(&["rechnung".to_string(), "mä".to_string()], true);
        assert_eq!(fts, "\"rechnung\" AND \"mä\"*");
    }

    #[test]
    fn drops_terms_without_alphanumeric_content() {
        // Punctuation-only terms tokenize to an empty phrase and would raise
        // an FTS MATCH syntax error.
        let fts = build_fts_query(&["@@@".to_string(), "rechnung".to_string()], true);
        assert_eq!(fts, "\"rechnung\"*");
        assert_eq!(build_fts_query(&["...".to_string()], true), "");
    }
}
