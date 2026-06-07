use crate::model::DepositEvent;
use anyhow::Result;
use std::path::Path;

/// Infer file format from the file extension.
pub fn infer_format(path: &str) -> FileFormat {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "json" => FileFormat::Json,
        "csv" => FileFormat::Csv,
        _ => FileFormat::Jsonl,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Json,
    Jsonl,
    Csv,
}

/// Serialize a deposit event to the given format.
pub fn serialize_event(event: &DepositEvent, format: FileFormat) -> Result<String> {
    match format {
        FileFormat::Json => Ok(serde_json::to_string_pretty(event)?),
        FileFormat::Jsonl => Ok(serde_json::to_string(event)?),
        FileFormat::Csv => {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(false)
                .from_writer(Vec::new());
            writer.serialize((
                &event.event_id,
                &event.event,
                event.version,
                &event.occurred_at,
                &event.data.tx_id,
                &event.data.caip2,
                &event.data.symbol,
                &event.data.address,
                event.data.block_number,
                event.data.log_index,
                &event.data.amount,
                &event.data.sender,
                event.data.confirmations,
                &event.data.timestamp,
            ))?;
            let bytes = writer.into_inner()?;
            Ok(String::from_utf8(bytes)?
                .trim_end_matches(['\r', '\n'])
                .to_string())
        }
    }
}

/// Serialize multiple events to the given format.
pub fn serialize_events(events: &[DepositEvent], format: FileFormat) -> Result<String> {
    match format {
        FileFormat::Json => Ok(serde_json::to_string_pretty(events)?),
        FileFormat::Jsonl => {
            let lines: Vec<String> = events
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(lines.join("\n"))
        }
        FileFormat::Csv => {
            let lines = events
                .iter()
                .map(|e| serialize_event(e, FileFormat::Csv))
                .collect::<Result<Vec<_>, _>>()?;
            if lines.is_empty() {
                Ok(String::new())
            } else {
                Ok(format!("{}\n", lines.join("\n")))
            }
        }
    }
}
