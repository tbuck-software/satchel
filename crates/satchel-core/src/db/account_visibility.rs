//! Account attention policy. Sync and direct account access deliberately ignore it.
//!
//! SQLite's local calendar clock is the authority for all consumers, including
//! the frontend status snapshot. Dates, rather than 24-hour durations, expire
//! overrides correctly across daylight-saving changes.
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

pub const KEY_PREFIX: &str = "account.visibility.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/db/account_visibility/")
)]
#[serde(default)]
pub struct AccountVisibilityPolicy {
    pub mode: String,
    pub weekdays: Vec<u8>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub override_date: Option<String>,
    pub override_visible: Option<bool>,
}

impl Default for AccountVisibilityPolicy {
    fn default() -> Self {
        Self {
            mode: "always".into(),
            weekdays: vec![1, 2, 3, 4, 5],
            from: None,
            to: None,
            override_date: None,
            override_visible: None,
        }
    }
}

impl AccountVisibilityPolicy {
    pub fn valid(&self) -> bool {
        fn valid_time(value: &str) -> bool {
            value.is_ascii()
                && value.len() == 5
                && value.as_bytes()[2] == b':'
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b':')
                && value[..2].parse::<u8>().is_ok_and(|v| v < 24)
                && value[3..].parse::<u8>().is_ok_and(|v| v < 60)
        }
        matches!(self.mode.as_str(), "always" | "schedule" | "manual")
            && self.weekdays.iter().all(|day| *day <= 6)
            && self.from.as_deref().is_none_or(valid_time)
            && self.to.as_deref().is_none_or(valid_time)
            && match (&self.from, &self.to) {
                (Some(from), Some(to)) => from < to,
                _ => true,
            }
            && self
                .override_date
                .as_deref()
                .is_none_or(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok())
    }
}

/// The expression must be a trusted SQL column name, never user input.
/// The uncorrelated subquery evaluates each saved policy once per statement.
pub fn visible_account_sql(account_column: &str) -> String {
    format!(
        "{account_column} NOT IN ({})",
        resting_accounts_sql("datetime('now', 'localtime')")
    )
}

fn resting_accounts_sql(clock: &str) -> String {
    format!(
        "SELECT substr(key, {start}) FROM app_settings
         WHERE key GLOB 'account.visibility.*' AND json_valid(value)
         AND NOT CASE
           WHEN json_extract(value, '$.override_date') = date({clock})
             AND json_type(value, '$.override_visible') IN ('true', 'false')
             THEN json_extract(value, '$.override_visible')
           WHEN COALESCE(json_extract(value, '$.mode'), 'always') = 'always' THEN 1
           WHEN json_extract(value, '$.mode') = 'schedule' THEN
             EXISTS (SELECT 1 FROM json_each(app_settings.value, '$.weekdays')
                     WHERE json_each.value = CAST(strftime('%w', {clock}) AS INTEGER))
             AND strftime('%H:%M', {clock}) >= COALESCE(json_extract(value, '$.from'), '00:00')
             AND strftime('%H:%M', {clock}) < COALESCE(json_extract(value, '$.to'), '24:00')
           ELSE 0 END",
        start = KEY_PREFIX.len() + 1,
    )
}

pub async fn is_visible(pool: &SqlitePool, account_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(&format!("SELECT {}", visible_account_sql("?")))
        .bind(account_id)
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn visible_at(policy: serde_json::Value, local_time: &str) -> bool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE app_settings (key TEXT, value TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO app_settings VALUES ('account.visibility.work', ?)")
            .bind(policy.to_string())
            .execute(&pool)
            .await
            .unwrap();
        let sql = format!("SELECT 'work' NOT IN ({})", resting_accounts_sql("?1"));
        sqlx::query_scalar(&sql)
            .bind(local_time)
            .fetch_one(&pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn weekday_and_exclusive_time_edges() {
        let policy =
            serde_json::json!({"mode":"schedule","weekdays":[1,3],"from":"09:00","to":"17:00"});
        for (time, expected) in [
            ("2026-10-26 08:59:59", false),
            ("2026-10-26 09:00:00", true),
            ("2026-10-26 16:59:59", true),
            ("2026-10-26 17:00:00", false),
            ("2026-10-27 12:00:00", false),
            ("2026-10-28 12:00:00", true),
        ] {
            assert_eq!(visible_at(policy.clone(), time).await, expected, "{time}");
        }
        assert!(
            visible_at(
                serde_json::json!({"mode":"schedule","weekdays":[1]}),
                "2026-10-26 23:59:59"
            )
            .await
        );
        assert!(
            !visible_at(
                serde_json::json!({"mode":"schedule","weekdays":[]}),
                "2026-10-26 12:00:00"
            )
            .await
        );
    }

    #[tokio::test]
    async fn overrides_expire_at_calendar_midnight_including_dst_days() {
        // Berlin's spring/fall transition days contain 23/25 hours. Neither
        // elapsed duration nor a UTC date participates in the expiry rule.
        for date in ["2026-03-29", "2026-10-25"] {
            let policy =
                serde_json::json!({"mode":"manual","override_date":date,"override_visible":true});
            for time in ["00:00:00", "01:59:59", "02:30:00", "03:00:00", "23:59:59"] {
                assert!(visible_at(policy.clone(), &format!("{date} {time}")).await);
            }
            let next = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .unwrap()
                .succ_opt()
                .unwrap();
            assert!(!visible_at(policy, &format!("{next} 00:00:00")).await);
        }
        assert!(!visible_at(serde_json::json!({"mode":"always","override_date":"2026-10-26","override_visible":false}), "2026-10-26 12:00:00").await);
        assert!(visible_at(serde_json::json!({"mode":"always","override_date":"2026-10-26","override_visible":false}), "2026-10-27 00:00:00").await);
    }

    #[tokio::test]
    async fn daylight_saving_changes_use_local_calendar_dates_and_wall_times() {
        let berlin = chrono_tz::Europe::Berlin;
        for (start, end, hours) in [
            ("2026-03-28T23:00:00Z", "2026-03-29T22:00:00Z", 23),
            ("2026-10-24T22:00:00Z", "2026-10-25T23:00:00Z", 25),
        ] {
            let start = chrono::DateTime::parse_from_rfc3339(start)
                .unwrap()
                .with_timezone(&berlin);
            let end = chrono::DateTime::parse_from_rfc3339(end)
                .unwrap()
                .with_timezone(&berlin);
            assert_eq!((end - start).num_hours(), hours);
            let policy = serde_json::json!({"mode":"manual","override_date":start.format("%Y-%m-%d").to_string(),"override_visible":true});
            assert!(
                visible_at(
                    policy.clone(),
                    &start.format("%Y-%m-%d %H:%M:%S").to_string()
                )
                .await
            );
            assert!(!visible_at(policy, &end.format("%Y-%m-%d %H:%M:%S").to_string()).await);
        }
        let policy =
            serde_json::json!({"mode":"schedule","weekdays":[0],"from":"02:00","to":"03:00"});
        // The repeated hour is in the window both times; no elapsed-hour math.
        for instant in ["2026-10-25T00:30:00Z", "2026-10-25T01:30:00Z"] {
            let local = chrono::DateTime::parse_from_rfc3339(instant)
                .unwrap()
                .with_timezone(&berlin);
            assert!(
                visible_at(
                    policy.clone(),
                    &local.format("%Y-%m-%d %H:%M:%S").to_string()
                )
                .await
            );
        }
        for instant in ["2026-03-29T00:59:59Z", "2026-03-29T01:00:00Z"] {
            let local = chrono::DateTime::parse_from_rfc3339(instant)
                .unwrap()
                .with_timezone(&berlin);
            assert!(
                !visible_at(
                    policy.clone(),
                    &local.format("%Y-%m-%d %H:%M:%S").to_string()
                )
                .await
            );
        }
    }

    #[test]
    fn validates_times_without_panicking_on_unicode() {
        let mut policy = AccountVisibilityPolicy::default();
        assert!(policy.valid());
        for invalid in ["24:00", "09:60", "+1:00", "x", "é:00", "🕘x"] {
            policy.from = Some(invalid.into());
            assert!(!policy.valid());
        }
        policy.from = Some("17:00".into());
        policy.to = Some("09:00".into());
        assert!(!policy.valid());
    }
}
