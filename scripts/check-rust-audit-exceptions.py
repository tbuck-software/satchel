"""Fail CI when a RustSec audit exception has passed its review deadline."""

from datetime import date, datetime, timezone
from pathlib import Path
import tomllib


REVIEW_DEADLINES = {"RUSTSEC-2023-0071": date(2026, 12, 31)}


def invalid_exceptions(today: date, config_path: Path) -> list[str]:
    if not config_path.exists():
        return []
    config = tomllib.loads(config_path.read_text())
    ignored = config.get("advisories", {}).get("ignore", [])
    return [
        advisory
        for advisory in ignored
        if advisory not in REVIEW_DEADLINES or today > REVIEW_DEADLINES[advisory]
    ]


if __name__ == "__main__":
    invalid = invalid_exceptions(datetime.now(timezone.utc).date(), Path(".cargo/audit.toml"))
    if invalid:
        raise SystemExit(
            f"RustSec exception missing a current review deadline: {', '.join(invalid)}. "
            "Review it and set a deadline, or remove the exception."
        )
