use crate::embed::TierConfig;
use std::sync::OnceLock;
use tiktoken_rs::CoreBPE;

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkSpec {
    pub text: String,
    pub chunk_index: i32,
    pub chunk_type: String, // "primary" | "detail"
}

fn cl100k_bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| match tiktoken_rs::cl100k_base() {
        Ok(bpe) => bpe,
        Err(err) => panic!("failed to load cl100k tokenizer (tiktoken-rs): {err}"),
    })
}

pub fn count_tokens(text: &str) -> usize {
    cl100k_bpe().encode_with_special_tokens(text).len()
}

pub fn truncate_to_max_tokens(text: &str, max_tokens: usize, context_info: &str) -> String {
    let bpe = cl100k_bpe();
    let ids = bpe.encode_with_special_tokens(text);
    if ids.len() > max_tokens {
        let truncated_ids = &ids[..max_tokens];
        let decoded = bpe
            .decode(truncated_ids.to_vec())
            .unwrap_or_else(|_| text.chars().take(max_tokens * 4).collect());
        eprintln!(
            "Warning: {} was truncated from {} to {} tokens",
            context_info,
            ids.len(),
            max_tokens
        );
        decoded
    } else {
        text.to_string()
    }
}

fn split_paragraph_to_sentences(para: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<char> = para.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if (c == '.' || c == '?' || c == '!')
            && (i + 1 == chars.len() || chars[i + 1].is_whitespace())
        {
            let sent: String = chars[start..=i].iter().collect();
            sentences.push(sent);
            start = i + 1;
        }
        i += 1;
    }
    if start < chars.len() {
        let sent: String = chars[start..].iter().collect();
        sentences.push(sent);
    }
    sentences
}

fn split_sentence_to_words(sent: &str, target_max: usize) -> Vec<String> {
    let words: Vec<&str> = sent.split_whitespace().collect();
    let mut word_units = Vec::new();
    let mut current_words = Vec::new();
    let mut current_tokens = 0;
    for word in words {
        let word_tokens = count_tokens(word);
        if current_tokens + word_tokens + (if current_words.is_empty() { 0 } else { 1 })
            > target_max
            && !current_words.is_empty()
        {
            word_units.push(current_words.join(" "));
            current_words.clear();
            current_tokens = 0;
        }
        let sep = if current_words.is_empty() { 0 } else { 1 };
        current_words.push(word);
        current_tokens += word_tokens + sep;
    }
    if !current_words.is_empty() {
        word_units.push(current_words.join(" "));
    }
    word_units
}

fn split_to_units(text: &str, target_max: usize) -> Vec<String> {
    let mut units = Vec::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        let para_tokens = count_tokens(para);
        if para_tokens <= target_max {
            units.push(para.to_string());
        } else {
            let sentences = split_paragraph_to_sentences(para);
            for sent in sentences {
                let sent = sent.trim();
                if sent.is_empty() {
                    continue;
                }
                let sent_tokens = count_tokens(sent);
                if sent_tokens <= target_max {
                    units.push(sent.to_string());
                } else {
                    let word_units = split_sentence_to_words(sent, target_max);
                    for wu in word_units {
                        let wu = wu.trim();
                        if !wu.is_empty() {
                            units.push(wu.to_string());
                        }
                    }
                }
            }
        }
    }
    units
}

pub fn chunk_node_text(
    title: &str,
    summary: &str,
    detail: Option<&str>,
    config: &TierConfig,
) -> Vec<ChunkSpec> {
    let mut chunks = Vec::new();

    // 1. Primary Chunk (index 0, type "primary")
    let primary_raw = format!("{}\n\n{}", title, summary);
    let primary_text = truncate_to_max_tokens(&primary_raw, config.max_tokens, "primary chunk");
    chunks.push(ChunkSpec {
        text: primary_text,
        chunk_index: 0,
        chunk_type: "primary".to_string(),
    });

    // 2. Detail Chunks (index 1..n, type "detail")
    if let Some(detail_text) = detail {
        let detail_trimmed = detail_text.trim();
        if !detail_trimmed.is_empty() {
            let _target_min = config
                .chunk_target_tokens
                .first()
                .copied()
                .unwrap_or(config.max_tokens / 2);
            let target_max = config
                .chunk_target_tokens
                .get(1)
                .copied()
                .unwrap_or(config.max_tokens);
            let overlap_max = config
                .chunk_overlap_tokens
                .get(1)
                .copied()
                .or_else(|| config.chunk_overlap_tokens.first().copied())
                .unwrap_or(0);

            let units = split_to_units(detail_trimmed, target_max);
            let mut current_chunk_units: Vec<String> = Vec::new();
            let mut detail_chunks_text = Vec::new();

            for unit in units {
                let unit_tokens = count_tokens(&unit);

                let next_tokens = if current_chunk_units.is_empty() {
                    unit_tokens
                } else {
                    let mut temp = current_chunk_units.join("\n\n");
                    temp.push_str("\n\n");
                    temp.push_str(&unit);
                    count_tokens(&temp)
                };

                if next_tokens > target_max && !current_chunk_units.is_empty() {
                    let chunk_text = current_chunk_units.join("\n\n");
                    detail_chunks_text.push(chunk_text);

                    // Walk backwards to build overlap buffer
                    let mut overlap_units: Vec<String> = Vec::new();
                    let mut overlap_tokens = 0;
                    for prev in current_chunk_units.iter().rev() {
                        let prev_tokens = count_tokens(prev);
                        // Cap overlap size: ensure it doesn't consume all units of this chunk
                        if overlap_units.len() + 1 >= current_chunk_units.len() {
                            break;
                        }
                        if overlap_tokens + prev_tokens > overlap_max {
                            break;
                        }
                        overlap_units.push(prev.clone());
                        overlap_tokens += prev_tokens;
                    }
                    overlap_units.reverse();

                    current_chunk_units = overlap_units;
                }

                current_chunk_units.push(unit);
            }

            if !current_chunk_units.is_empty() {
                detail_chunks_text.push(current_chunk_units.join("\n\n"));
            }

            for (idx, text) in (1..).zip(detail_chunks_text) {
                let context = format!("detail chunk {}", idx);
                let final_text = truncate_to_max_tokens(&text, config.max_tokens, &context);
                chunks.push(ChunkSpec {
                    text: final_text,
                    chunk_index: idx,
                    chunk_type: "detail".to_string(),
                });
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::registry::tier_config;

    #[test]
    fn test_short_node_only_primary() -> Result<(), Box<dyn std::error::Error>> {
        let config = tier_config("light").ok_or("light config missing")?;
        let chunks = chunk_node_text("Short Title", "Short Summary", None, &config);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].chunk_type, "primary");
        assert_eq!(chunks[0].text, "Short Title\n\nShort Summary");
        Ok(())
    }

    #[test]
    fn test_long_detail_split() -> Result<(), Box<dyn std::error::Error>> {
        let config = tier_config("light").ok_or("light config missing")?;

        // Construct paragraphs that exceed standard targets
        let p1 = "This is a sentence. ".repeat(40); // around 160 tokens
        let p2 = "This is another sentence. ".repeat(40); // around 200 tokens
        let p3 = "This is a third sentence. ".repeat(40); // around 200 tokens

        let detail = format!("{}\n\n{}\n\n{}", p1, p2, p3);
        let chunks = chunk_node_text("Title", "Summary", Some(&detail), &config);

        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].chunk_type, "primary");

        // Assert chunk index continuity
        for (i, chunk) in chunks.iter().enumerate().skip(1) {
            assert_eq!(chunk.chunk_index, i as i32);
            assert_eq!(chunk.chunk_type, "detail");
        }
        Ok(())
    }

    #[test]
    fn test_gist_tokens_range() -> Result<(), Box<dyn std::error::Error>> {
        let config = tier_config("standard").ok_or("standard config missing")?;

        let target_min = config.chunk_target_tokens[0];
        let target_max = config.chunk_target_tokens[1];

        // Construct extremely long detail content to form multiple chunks
        let detail = "Here is some body text that will be chunked. ".repeat(200);
        let chunks = chunk_node_text("Title", "Summary", Some(&detail), &config);

        // First chunk index 0 is primary, rest are details
        assert!(chunks.len() > 2);

        // Check all detail chunks except the last one (which contains the remaining text)
        let detail_len = chunks.len();
        for (i, chunk) in chunks.iter().enumerate().take(detail_len - 1).skip(1) {
            let tokens = count_tokens(&chunk.text);
            assert!(
                tokens <= target_max,
                "Detail chunk {} tokens {} exceeded target_max {}",
                i,
                tokens,
                target_max
            );
            assert!(
                tokens >= target_min,
                "Detail chunk {} tokens {} was below target_min {}",
                i,
                tokens,
                target_min
            );
        }

        // For the last chunk, we only verify it does not exceed target_max
        let last_chunk = chunks.last().ok_or("no chunks found")?;
        let last_tokens = count_tokens(&last_chunk.text);
        assert!(
            last_tokens <= target_max,
            "Last detail chunk tokens {} exceeded target_max {}",
            last_tokens,
            target_max
        );
        Ok(())
    }

    #[test]
    fn test_hard_truncation() -> Result<(), Box<dyn std::error::Error>> {
        let mut config = tier_config("light").ok_or("light config missing")?;
        config.max_tokens = 50; // override for easy truncation testing

        let long_detail = "word ".repeat(200);
        let chunks = chunk_node_text("Title", "Summary", Some(&long_detail), &config);

        assert!(chunks.len() > 1);
        for chunk in chunks {
            let tokens = count_tokens(&chunk.text);
            assert!(
                tokens <= 50,
                "Chunk exceeded truncated limit of 50 tokens, got {}",
                tokens
            );
        }
        Ok(())
    }

    #[test]
    fn test_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
        let config = tier_config("light").ok_or("light config missing")?;

        // 1. Whitespace only detail
        let chunks_whitespace = chunk_node_text("Title", "Summary", Some("   \n  "), &config);
        assert_eq!(chunks_whitespace.len(), 1);

        // 2. Empty string detail
        let chunks_empty = chunk_node_text("Title", "Summary", Some(""), &config);
        assert_eq!(chunks_empty.len(), 1);

        // 3. Small configuration safety vectors (short config targets)
        let malformed_config = TierConfig {
            model_id: "test".to_string(),
            params_m: 0,
            dims: 10,
            max_tokens: 100,
            onnx_size_mb: 0,
            chunk_target_tokens: vec![],  // empty target
            chunk_overlap_tokens: vec![], // empty overlap
            rules: serde_json::Value::Null,
            fallback_model_id: None,
        };
        let detail = "Some detail text for fallback test.".repeat(10);
        let chunks_fallback = chunk_node_text("Title", "Summary", Some(&detail), &malformed_config);
        assert!(!chunks_fallback.is_empty());
        Ok(())
    }
}
