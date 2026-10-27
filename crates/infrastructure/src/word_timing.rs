use parakeet_domain::word_timing::RecognizedWord;

struct WordGroup {
    bytes: Vec<u8>,
    range: Option<(i64, i64)>,
}
pub struct TokenTiming {
    pub bytes: Vec<u8>,
    pub start_ms: i64,
    pub end_ms: i64,
}
/// Group decoded subword bytes at whitespace; never interpolate a timestamp.
/// Invalid token ranges or a token spanning multiple words make those words
/// untimed. A reconstruction mismatch leaves every original word untimed.
pub fn words(
    text: &str,
    tokens: &[TokenTiming],
    chunk_origin: i64,
    chunk_duration: i64,
    meeting_offset: Option<i64>,
) -> Vec<RecognizedWord> {
    let mut groups: Vec<WordGroup> = vec![];
    let mut bytes = vec![];
    let mut range = None;
    let mut valid = true;
    for token in tokens {
        let groups_in_token = token
            .bytes
            .split(|b| b.is_ascii_whitespace())
            .filter(|b| !b.is_empty())
            .count();
        let token_valid = groups_in_token <= 1
            && token.start_ms >= 0
            && token.end_ms > token.start_ms
            && token.end_ms <= chunk_duration;
        for &byte in &token.bytes {
            if byte.is_ascii_whitespace() {
                if !bytes.is_empty() {
                    groups.push(WordGroup {
                        bytes: std::mem::take(&mut bytes),
                        range: if valid { range } else { None },
                    });
                    range = None;
                    valid = true;
                }
            } else {
                bytes.push(byte);
                valid &= token_valid;
                range = Some(match range {
                    Some((a, b)) => (a.min(token.start_ms), b.max(token.end_ms)),
                    None => (token.start_ms, token.end_ms),
                });
            }
        }
    }
    if !bytes.is_empty() {
        groups.push(WordGroup {
            bytes,
            range: if valid { range } else { None },
        });
    }
    let decoded = groups
        .iter()
        .map(|group| String::from_utf8(group.bytes.clone()))
        .collect::<Result<Vec<_>, _>>();
    let original = text.split_whitespace().collect::<Vec<_>>();
    let matching = decoded
        .as_ref()
        .is_ok_and(|v| v.iter().map(String::as_str).eq(original.iter().copied()));
    original
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let local = matching
                .then(|| groups[i].range)
                .flatten()
                .and_then(|(a, b)| {
                    Some((chunk_origin.checked_add(a)?, chunk_origin.checked_add(b)?))
                });
            let common = local.and_then(|(a, b)| {
                Some((
                    meeting_offset?.checked_add(a)?,
                    meeting_offset?.checked_add(b)?,
                ))
            });
            RecognizedWord {
                text: text.into(),
                local_start_ms: local.map(|x| x.0),
                local_end_ms: local.map(|x| x.1),
                start_ms: common.map(|x| x.0),
                end_ms: common.map(|x| x.1),
                timing_quality: if local.is_some() {
                    "experimental_whisper_token_estimate"
                } else {
                    "untimed"
                }
                .into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(bytes: &[u8], a: i64, b: i64) -> TokenTiming {
        TokenTiming {
            bytes: bytes.into(),
            start_ms: a,
            end_ms: b,
        }
    }
    #[test]
    fn preserves_subwords_repeats_utf8_and_actual_offsets() {
        let v = words(
            " again again café",
            &[
                t(b" again", 0, 100),
                t(b" again", 100, 200),
                t(b" caf\xc3", 200, 250),
                t(b"\xa9", 250, 300),
            ],
            1000,
            500,
            Some(2000),
        );
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].text, v[1].text);
        assert_eq!(v[2].text, "café");
        assert_eq!(v[2].start_ms, Some(3200));
        assert_eq!(v[2].end_ms, Some(3300));
    }
    #[test]
    fn missing_invalid_ambiguous_and_mismatched_timings_stay_untimed() {
        let v = words(
            "one two three",
            &[t(b"one two", 0, 100), t(b" three", 200, 200)],
            0,
            500,
            Some(0),
        );
        assert!(v.iter().all(|w| w.start_ms.is_none()));
        let v = words(
            "original words",
            &[t(b"different words", 0, 100)],
            0,
            500,
            Some(0),
        );
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|w| w.local_start_ms.is_none()));
        let v = words("late", &[t(b"late", 0, 100)], 1000, 500, None);
        assert_eq!(v[0].local_start_ms, Some(1000));
        assert_eq!(v[0].start_ms, None);
    }
}
