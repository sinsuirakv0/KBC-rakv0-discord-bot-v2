//! Event data共通の日時Modelと表示処理を定義する。

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduleHeader {
    pub(crate) start_date: String,
    pub(crate) start_time: String,
    pub(crate) end_date: String,
    pub(crate) end_time: String,
    pub(crate) min_version: String,
    pub(crate) max_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DateRange {
    pub(crate) start: String,
    pub(crate) end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimeBlock {
    pub(crate) date_ranges: Vec<DateRange>,
    pub(crate) month_days: Vec<i64>,
    pub(crate) weekdays: Vec<String>,
    pub(crate) time_ranges: Vec<[String; 2]>,
}

pub(crate) fn parse_header_date(date_text: &str, time_text: &str) -> Result<DateTime<Utc>, String> {
    let date = format!("{:0>8}", date_text.trim());
    let time = format!("{:0>4}", time_text.trim());
    if date.len() != 8 || time.len() != 4 {
        return Err("date or time has an invalid length".to_owned());
    }
    let year = parse_part(&date, 0, 4)?;
    let month = parse_part(&date, 4, 6)? as u32;
    let day = parse_part(&date, 6, 8)? as u32;
    let hour = parse_part(&time, 0, 2)? as u32;
    let minute = parse_part(&time, 2, 4)? as u32;
    let local_date = NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| "date is outside the calendar".to_owned())?;
    let local_time = NaiveTime::from_hms_opt(hour, minute, 0)
        .ok_or_else(|| "time is outside the clock".to_owned())?;
    let timezone =
        FixedOffset::east_opt(9 * 60 * 60).ok_or_else(|| "JST offset is invalid".to_owned())?;
    timezone
        .from_local_datetime(&local_date.and_time(local_time))
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| "date and time are ambiguous".to_owned())
}

pub(crate) fn validate_header(header: &ScheduleHeader) -> Result<(), String> {
    parse_header_date(&header.start_date, &header.start_time)?;
    parse_header_date(&header.end_date, &header.end_time)?;
    Ok(())
}

pub(crate) fn format_jst_full(date: DateTime<Utc>) -> String {
    let value = date.with_timezone(&jst());
    format!(
        "{}年{}月{}日({}) {:02}:{:02}",
        value.year(),
        value.month(),
        value.day(),
        weekday_japanese(value.weekday()),
        value.hour(),
        value.minute()
    )
}

pub(crate) fn format_jst_short(date: DateTime<Utc>) -> String {
    let value = date.with_timezone(&jst());
    format!(
        "{}/{}({}) {:02}:{:02}",
        value.month(),
        value.day(),
        weekday_japanese(value.weekday()),
        value.hour(),
        value.minute()
    )
}

pub(crate) fn format_duration(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let mut remaining_minutes = (end - start).num_minutes().max(0);
    let days = remaining_minutes / 1_440;
    remaining_minutes %= 1_440;
    let hours = remaining_minutes / 60;
    let minutes = remaining_minutes % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    format!(
        "<{}>",
        if parts.is_empty() {
            "0m".to_owned()
        } else {
            parts.concat()
        }
    )
}

pub(crate) fn format_time_block(block: &TimeBlock) -> String {
    let day_part = if !block.weekdays.is_empty() {
        let weekdays = block
            .weekdays
            .iter()
            .map(|weekday| weekday_name(weekday))
            .collect::<Vec<_>>()
            .join("・");
        format!("毎週{weekdays}曜")
    } else if !block.month_days.is_empty() {
        let days = block
            .month_days
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!("毎月{days}日")
    } else if !block.date_ranges.is_empty() {
        block
            .date_ranges
            .iter()
            .map(|range| {
                format!(
                    "{}~{}",
                    format_date_range_point(&range.start),
                    format_date_range_point(&range.end)
                )
            })
            .collect::<Vec<_>>()
            .join(" / ")
    } else {
        "毎日".to_owned()
    };
    let time_part = if block.time_ranges.is_empty() {
        "終日".to_owned()
    } else {
        block
            .time_ranges
            .iter()
            .map(|range| {
                format!(
                    "{}~{}",
                    format_minutes(parse_time_minutes(&range[0])),
                    format_minutes(parse_time_minutes(&range[1]))
                )
            })
            .collect::<Vec<_>>()
            .join("、")
    };
    format!("{day_part}  {time_part}")
}

fn parse_part(value: &str, start: usize, end: usize) -> Result<i32, String> {
    value
        .get(start..end)
        .ok_or_else(|| "date or time is not ASCII".to_owned())?
        .parse()
        .map_err(|_| "date or time contains a non-number".to_owned())
}

fn parse_time_minutes(value: &str) -> i64 {
    let padded = format!("{:0>4}", value.trim());
    let hour = padded
        .get(0..2)
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    let minute = padded
        .get(2..4)
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    hour * 60 + minute
}

fn format_minutes(value: i64) -> String {
    if value >= 1_440 {
        return "24:00".to_owned();
    }
    format!("{:02}:{:02}", value / 60, value % 60)
}

fn format_date_range_point(value: &str) -> String {
    let mut parts = value.split_whitespace();
    let month_day = format!("{:0>4}", parts.next().unwrap_or("0"));
    let time = format!("{:0>4}", parts.next().unwrap_or("0"));
    let month = month_day
        .get(0..2)
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let day = month_day
        .get(2..4)
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let hour = time.get(0..2).unwrap_or("00");
    let minute = time.get(2..4).unwrap_or("00");
    format!("{month}/{day} {hour}:{minute}")
}

fn jst() -> FixedOffset {
    FixedOffset::east_opt(9 * 60 * 60).expect("JST is a valid fixed offset")
}

fn weekday_japanese(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Sun => "日",
        chrono::Weekday::Mon => "月",
        chrono::Weekday::Tue => "火",
        chrono::Weekday::Wed => "水",
        chrono::Weekday::Thu => "木",
        chrono::Weekday::Fri => "金",
        chrono::Weekday::Sat => "土",
    }
}

fn weekday_name(value: &str) -> &str {
    match value {
        "Sun" => "日",
        "Mon" => "月",
        "Tue" => "火",
        "Wed" => "水",
        "Thu" => "木",
        "Fri" => "金",
        "Sat" => "土",
        _ => value,
    }
}
