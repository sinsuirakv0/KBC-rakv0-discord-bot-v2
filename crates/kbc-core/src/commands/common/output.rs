//! DiscordのMessage長を考慮して有限なCommand出力へ積む。

use kbc_protocol::CoreActionData;

use crate::command::{CommandExecutionError, CommandOutput};

pub(crate) const DISCORD_TEXT_CHUNK_SIZE: usize = 1_800;

pub(crate) fn push_message(
    output: &mut CommandOutput,
    channel_id: &str,
    content: impl Into<String>,
) -> Result<(), CommandExecutionError> {
    output.push(CoreActionData::SendMessage {
        channel_id: channel_id.to_owned(),
        content: content.into(),
    })
}

pub(crate) fn push_chunked(
    output: &mut CommandOutput,
    channel_id: &str,
    text: &str,
    language: &str,
) -> Result<(), CommandExecutionError> {
    let opening = if language.is_empty() {
        "```\n".to_owned()
    } else {
        format!("```{language}\n")
    };
    for chunk in split_text_into_chunks(text, DISCORD_TEXT_CHUNK_SIZE) {
        push_message(output, channel_id, format!("{opening}{chunk}\n```"))?;
    }
    Ok(())
}

fn split_text_into_chunks(text: &str, max_length: usize) -> Vec<String> {
    debug_assert!(max_length > 0);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_length = 0;

    for line in text.split('\n') {
        let line_length = utf16_length(line);
        if line_length > max_length {
            flush(&mut chunks, &mut current, &mut current_length);
            split_long_line(line, max_length, &mut chunks);
            continue;
        }

        let separator_length = usize::from(!current.is_empty());
        if !current.is_empty() && current_length + separator_length + line_length > max_length {
            flush(&mut chunks, &mut current, &mut current_length);
        }
        if !current.is_empty() {
            current.push('\n');
            current_length += 1;
        }
        current.push_str(line);
        current_length += line_length;
    }
    flush(&mut chunks, &mut current, &mut current_length);
    chunks
}

fn split_long_line(line: &str, max_length: usize, chunks: &mut Vec<String>) {
    let mut current = String::new();
    let mut current_length = 0;
    for character in line.chars() {
        let character_length = character.len_utf16();
        if !current.is_empty() && current_length + character_length > max_length {
            chunks.push(std::mem::take(&mut current));
            current_length = 0;
        }
        current.push(character);
        current_length += character_length;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
}

fn flush(chunks: &mut Vec<String>, current: &mut String, current_length: &mut usize) {
    if !current.is_empty() {
        chunks.push(std::mem::take(current));
    }
    *current_length = 0;
}

fn utf16_length(value: &str) -> usize {
    value.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::split_text_into_chunks;

    #[test]
    fn splits_an_overlong_line_without_losing_text() {
        let text = "x".repeat(3_701);
        let chunks = split_text_into_chunks(&text, 1_800);

        assert_eq!(
            chunks.iter().map(String::len).collect::<Vec<_>>(),
            [1_800, 1_800, 101]
        );
        assert_eq!(chunks.concat(), text);
    }
}
