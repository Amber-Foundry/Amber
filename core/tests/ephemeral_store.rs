use std::error::Error;

use amber_lib::ephemeral::{EphemeralAttachmentSummary, EphemeralChunk, EphemeralChunkStore};
use amber_lib::ingest::job::ImportChunkSpec;

#[test]
fn ephemeral_chunk_store_populate_detach_and_clear_session() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();

    let session1 = "session-alpha";
    let session2 = "session-beta";
    let attach1 = "doc-101";
    let attach2 = "doc-102";

    assert_eq!(store.total_attachments(), 0);
    assert_eq!(store.total_chunks(), 0);

    let summary1 = EphemeralAttachmentSummary {
        session_id: session1.to_string(),
        attachment_id: attach1.to_string(),
        source_name: "doc1.pdf".to_string(),
        file_path: "/tmp/doc1.pdf".to_string(),
        total_chunks: 2,
        total_tokens: 500,
        page_count: 5,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed: true,
    };

    let chunks1 = vec![
        EphemeralChunk {
            chunk_index: 0,
            text: "Section 1 content for doc 1".to_string(),
            token_count: 250,
            heading_context: Some("Heading 1".to_string()),
            source_page_indices: vec![0, 1],
            embedding: vec![0.1f32; 384],
            ocr_confidence: None,
            tables_unstructured: false,
        },
        EphemeralChunk {
            chunk_index: 1,
            text: "Section 2 content for doc 1".to_string(),
            token_count: 250,
            heading_context: Some("Heading 2".to_string()),
            source_page_indices: vec![2, 3, 4],
            embedding: vec![0.2f32; 384],
            ocr_confidence: None,
            tables_unstructured: false,
        },
    ];

    let summary2 = EphemeralAttachmentSummary {
        session_id: session1.to_string(),
        attachment_id: attach2.to_string(),
        source_name: "doc2.pdf".to_string(),
        file_path: "/tmp/doc2.pdf".to_string(),
        total_chunks: 1,
        total_tokens: 300,
        page_count: 2,
        ocr_confidence: Some(0.92),
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed: true,
    };

    let chunks2 = vec![EphemeralChunk {
        chunk_index: 0,
        text: "Single chunk content for doc 2".to_string(),
        token_count: 300,
        heading_context: None,
        source_page_indices: vec![0, 1],
        embedding: vec![0.3f32; 384],
        ocr_confidence: Some(0.92),
        tables_unstructured: false,
    }];

    let summary_beta = EphemeralAttachmentSummary {
        session_id: session2.to_string(),
        attachment_id: "doc-201".to_string(),
        source_name: "doc_beta.pdf".to_string(),
        file_path: "/tmp/doc_beta.pdf".to_string(),
        total_chunks: 1,
        total_tokens: 150,
        page_count: 1,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed: true,
    };

    let chunks_beta = vec![EphemeralChunk {
        chunk_index: 0,
        text: "Beta session chunk".to_string(),
        token_count: 150,
        heading_context: None,
        source_page_indices: vec![0],
        embedding: vec![0.4f32; 384],
        ocr_confidence: None,
        tables_unstructured: false,
    }];

    // 1. Populate attach
    store.insert_attachment(summary1.clone(), chunks1.clone());
    store.insert_attachment(summary2.clone(), chunks2.clone());
    store.insert_attachment(summary_beta, chunks_beta);

    assert_eq!(store.total_attachments(), 3);
    assert_eq!(store.total_chunks(), 4);
    assert!(store.has_attachment(session1, attach1));
    assert!(store.has_attachment(session1, attach2));

    // Verify session chunks aggregation
    let alpha_chunks = store.get_session_chunks(session1);
    assert_eq!(alpha_chunks.len(), 3);

    // 2. Detach single document
    let removed = store.remove_attachment(session1, attach1);
    assert!(removed, "expected successful removal of attach1");
    assert!(!store.has_attachment(session1, attach1));
    assert!(store.has_attachment(session1, attach2));
    assert_eq!(store.total_attachments(), 2);

    // 3. Clear session
    let cleared_count = store.clear_session(session1);
    assert_eq!(
        cleared_count, 1,
        "cleared remaining attachment in session-alpha"
    );
    assert!(!store.has_attachment(session1, attach2));
    assert!(
        store.has_attachment(session2, "doc-201"),
        "session-beta attachment must remain"
    );

    Ok(())
}

#[test]
fn embeddings_computed_once_per_chunk_not_recomputed_per_turn() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();
    let session_id = "session-compute-test";
    let attachment_id = "attach-compute-1";

    let specs = vec![
        ImportChunkSpec {
            chunk_index: 0,
            text: "First test chunk for single compute test".to_string(),
            token_count: 100,
            heading_context: Some("Test".to_string()),
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        },
        ImportChunkSpec {
            chunk_index: 1,
            text: "Second test chunk for single compute test".to_string(),
            token_count: 120,
            heading_context: Some("Test".to_string()),
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        },
    ];

    // Compute embeddings ONCE on attach
    let (embeddings, embeddings_computed) =
        amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&specs);
    assert_eq!(embeddings.len(), 2);
    assert_eq!(embeddings[0].len(), 384);

    let ephemeral_chunks: Vec<EphemeralChunk> = specs
        .into_iter()
        .zip(embeddings)
        .map(|(spec, vec)| EphemeralChunk {
            chunk_index: spec.chunk_index,
            text: spec.text,
            token_count: spec.token_count,
            heading_context: spec.heading_context,
            source_page_indices: spec.source_page_indices,
            embedding: vec,
            ocr_confidence: spec.ocr_confidence,
            tables_unstructured: spec.tables_unstructured,
        })
        .collect();

    let summary = EphemeralAttachmentSummary {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        source_name: "compute_test.pdf".to_string(),
        file_path: "/tmp/compute_test.pdf".to_string(),
        total_chunks: 2,
        total_tokens: 220,
        page_count: 1,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed,
    };

    // Attach document (1st and ONLY compute)
    store.insert_attachment(summary, ephemeral_chunks);
    assert_eq!(store.embedding_compute_count(session_id, attachment_id), 1);

    // Simulate multiple turns accessing chunks from store
    for _turn in 1..=5 {
        let chunks = store
            .get_attachment_chunks(session_id, attachment_id)
            .ok_or("expected chunks for session")?;
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].embedding.len(), 384);

        // Verify compute count remains strictly 1 (never recomputed per turn)
        assert_eq!(
            store.embedding_compute_count(session_id, attachment_id),
            1,
            "embeddings must not be recomputed per turn"
        );
    }

    Ok(())
}

#[test]
fn vector_retrieval_finds_deep_page_fact_and_keeps_token_count_low() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();
    let session_id = "session-retrieval-test";
    let attachment_id = "doc-constitution-26th";

    // Simulate a 20-page document where page 18 (index 17) contains Amendment XXVI
    let mut chunks = Vec::new();
    for i in 0..20 {
        let (text, heading) = if i == 17 {
            (
                "Amendment XXVI (Amendment 26 - Voting Age): The right of citizens of the United States, who are eighteen years of age or older, to vote shall not be denied or abridged by the United States or by any State on account of age.".to_string(),
                Some("Amendment XXVI".to_string()),
            )
        } else {
            (
                format!("Constitution Section {} text about general legislative procedures and government structure.", i + 1),
                Some(format!("Article {}", i + 1)),
            )
        };

        let spec = ImportChunkSpec {
            chunk_index: i,
            text,
            token_count: 200,
            heading_context: heading,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![i],
        };
        chunks.push(spec);
    }

    let (embeddings, embeddings_computed) =
        amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&chunks);
    let ephemeral_chunks: Vec<EphemeralChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(spec, vec)| EphemeralChunk {
            chunk_index: spec.chunk_index,
            text: spec.text,
            token_count: spec.token_count,
            heading_context: spec.heading_context,
            source_page_indices: spec.source_page_indices,
            embedding: vec,
            ocr_confidence: spec.ocr_confidence,
            tables_unstructured: spec.tables_unstructured,
        })
        .collect();

    let summary = EphemeralAttachmentSummary {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        source_name: "constitution.pdf".to_string(),
        file_path: "/tmp/constitution.pdf".to_string(),
        total_chunks: 20,
        total_tokens: 4000,
        page_count: 20,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed,
    };

    store.insert_attachment(summary, ephemeral_chunks);

    // Query asking specifically about the 26th amendment
    let query_text = "What did the 26th Amendment do about voting age?";
    let query_vec = amber_lib::ephemeral::embed_query(query_text);

    let top_chunks = store.query_session_chunks(session_id, query_text, &query_vec, 5);

    // 1. Assert top_chunks returns top-K=5 chunks
    assert_eq!(top_chunks.len(), 5);

    // 2. Assert chunk 17 (Amendment XXVI on page 18) is present in retrieved chunks
    let has_target_chunk = top_chunks.iter().any(|c| c.chunk_index == 17);
    assert!(
        has_target_chunk,
        "Target chunk containing Amendment XXVI on page 18 must be retrieved"
    );

    // 3. Assert total retrieved token count is low (5 chunks * 200 tokens = 1000 tokens)
    let retrieved_token_count: usize = top_chunks.iter().map(|c| c.token_count).sum();
    assert!(
        retrieved_token_count <= 1200,
        "Retrieved token count ({} tokens) must stay well under the 4000 token full-doc ceiling",
        retrieved_token_count
    );

    Ok(())
}

#[test]
fn vector_retrieval_finds_amendment_xxiv_with_precision() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();
    let session_id = "session-amendments-test";
    let attachment_id = "doc-us-amendments";

    let amendments = vec![
        (12, "Amendment XII Passed by Congress December 9, 1803. Election of President and Vice President."),
        (17, "Amendment XVII Passed by Congress May 13, 1912. Popular election of Senators."),
        (24, "Amendment XXIV Passed by Congress August 27, 1962. Abolition of poll taxes in federal elections."),
        (26, "Amendment XXVI Passed by Congress March 23, 1971. Voting age lowered to 18."),
    ];

    let mut chunks = Vec::new();
    for (idx, (num, text)) in amendments.into_iter().enumerate() {
        let spec = ImportChunkSpec {
            chunk_index: idx,
            text: text.to_string(),
            token_count: 150,
            heading_context: Some(format!("Amendment {}", num)),
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![num],
        };
        chunks.push(spec);
    }

    let (embeddings, embeddings_computed) =
        amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&chunks);
    let ephemeral_chunks: Vec<EphemeralChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(spec, vec)| EphemeralChunk {
            chunk_index: spec.chunk_index,
            text: spec.text,
            token_count: spec.token_count,
            heading_context: spec.heading_context,
            source_page_indices: spec.source_page_indices,
            embedding: vec,
            ocr_confidence: spec.ocr_confidence,
            tables_unstructured: spec.tables_unstructured,
        })
        .collect();

    let summary = EphemeralAttachmentSummary {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        source_name: "AllAmendments_US.pdf".to_string(),
        file_path: "/tmp/AllAmendments_US.pdf".to_string(),
        total_chunks: 4,
        total_tokens: 600,
        page_count: 30,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed,
    };

    store.insert_attachment(summary, ephemeral_chunks);

    let query_text = "Abolition of poll taxes in federal elections Amendment XXIV";
    let query_vec = amber_lib::ephemeral::embed_query(query_text);

    let top_chunks = store.query_session_chunks(session_id, query_text, &query_vec, 3);
    assert!(!top_chunks.is_empty());
    assert_eq!(
        top_chunks[0].chunk_index, 2,
        "Amendment XXIV (index 2) must rank #1"
    );
    assert!(top_chunks[0].text.contains("Amendment XXIV"));

    Ok(())
}

#[test]
fn small_document_bypasses_retrieval() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();
    let session_id = "session-small-doc";
    let attachment_id = "doc-small-memo";

    let chunks = vec![
        ImportChunkSpec {
            chunk_index: 0,
            text: "Short memo paragraph 1.".to_string(),
            token_count: 200,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        },
        ImportChunkSpec {
            chunk_index: 1,
            text: "Short memo paragraph 2.".to_string(),
            token_count: 200,
            heading_context: None,
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![0],
        },
    ];

    let (embeddings, embeddings_computed) =
        amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&chunks);
    let ephemeral_chunks: Vec<EphemeralChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(spec, vec)| EphemeralChunk {
            chunk_index: spec.chunk_index,
            text: spec.text,
            token_count: spec.token_count,
            heading_context: spec.heading_context,
            source_page_indices: spec.source_page_indices,
            embedding: vec,
            ocr_confidence: spec.ocr_confidence,
            tables_unstructured: spec.tables_unstructured,
        })
        .collect();

    let summary = EphemeralAttachmentSummary {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        source_name: "small_memo.pdf".to_string(),
        file_path: "/tmp/small_memo.pdf".to_string(),
        total_chunks: 2,
        total_tokens: 400,
        page_count: 1,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed,
    };

    store.insert_attachment(summary, ephemeral_chunks);

    let (bypass, reason) = store.should_bypass_retrieval(session_id, "What does paragraph 1 say?");
    assert!(bypass, "Small document (400 tokens) must bypass retrieval");
    assert!(reason.contains("Small document fits fully"));

    Ok(())
}

#[test]
fn broad_question_triggers_whole_document_fallback() -> Result<(), Box<dyn Error>> {
    let mut store = EphemeralChunkStore::new();
    let session_id = "session-large-doc";
    let attachment_id = "doc-large-manual";

    let mut chunks = Vec::new();
    for i in 0..10 {
        let spec = ImportChunkSpec {
            chunk_index: i,
            text: format!("Manual section {} detailed text...", i + 1),
            token_count: 350,
            heading_context: Some(format!("Section {}", i + 1)),
            chunk_type: "import".to_string(),
            ocr_confidence: None,
            tables_unstructured: false,
            source_page_indices: vec![i],
        };
        chunks.push(spec);
    }

    let (embeddings, embeddings_computed) =
        amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&chunks);
    let ephemeral_chunks: Vec<EphemeralChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(spec, vec)| EphemeralChunk {
            chunk_index: spec.chunk_index,
            text: spec.text,
            token_count: spec.token_count,
            heading_context: spec.heading_context,
            source_page_indices: spec.source_page_indices,
            embedding: vec,
            ocr_confidence: spec.ocr_confidence,
            tables_unstructured: spec.tables_unstructured,
        })
        .collect();

    let summary = EphemeralAttachmentSummary {
        session_id: session_id.to_string(),
        attachment_id: attachment_id.to_string(),
        source_name: "large_manual.pdf".to_string(),
        file_path: "/tmp/large_manual.pdf".to_string(),
        total_chunks: 10,
        total_tokens: 3500,
        page_count: 10,
        ocr_confidence: None,
        prompt_injection_flagged: false,
        needs_ocr_models: false,
        embeddings_computed,
    };

    store.insert_attachment(summary, ephemeral_chunks);

    // Specific query should use selective top-K retrieval
    let (bypass1, _) =
        store.should_bypass_retrieval(session_id, "What is Section 5 detailed text?");
    assert!(
        !bypass1,
        "Specific question on 3500-token document must use smart top-K retrieval"
    );

    // Broad summarization query should trigger whole-document bypass
    let (bypass2, reason2) =
        store.should_bypass_retrieval(session_id, "Please summarize the entire document");
    assert!(
        bypass2,
        "Broad summarization query must trigger whole-document fallback"
    );
    assert!(reason2.contains("Broad document request detected"));

    // Assert punctuation-handling: tl;dr and bird's eye
    let (bypass_tldr, _) =
        store.should_bypass_retrieval(session_id, "Give me a tl;dr of the document");
    assert!(
        bypass_tldr,
        "tl;dr must trigger broad summarization fallback"
    );

    let (bypass_bird1, _) =
        store.should_bypass_retrieval(session_id, "Give me a birds eye view of the file");
    assert!(
        bypass_bird1,
        "birds eye view must trigger broad summarization fallback"
    );

    let (bypass_bird2, _) =
        store.should_bypass_retrieval(session_id, "Give me a bird's eye view of the file");
    assert!(
        bypass_bird2,
        "bird's eye view with apostrophe must trigger broad summarization fallback"
    );

    // Assert explicit whole-document scope overrides narrow target words ("code", "error")
    let (bypass_explicit_override, _) = store.should_bypass_retrieval(
        session_id,
        "Can you summarize the entire document? I need to understand how the code handles error handling across all sections.",
    );
    assert!(
        bypass_explicit_override,
        "Explicit 'entire document' scope must override narrow target words"
    );

    // Assert plural scope terms ("documents", "files")
    let (bypass_plural, _) =
        store.should_bypass_retrieval(session_id, "Give me takeaways from these documents");
    assert!(
        bypass_plural,
        "Plural scope terms like 'documents' must trigger broad summarization"
    );

    // Assert "overview of the code" triggers broad summarization
    let (bypass_code_overview, _) =
        store.should_bypass_retrieval(session_id, "Can you give me an overview of the code?");
    assert!(
        bypass_code_overview,
        "'overview of the code' must trigger broad summarization fallback"
    );

    // Assert localized section requests return false (Top-K Retrieval)
    let (bypass_localized1, _) =
        store.should_bypass_retrieval(session_id, "Please summarize chapter 3.");
    assert!(
        !bypass_localized1,
        "Localized 'summarize chapter 3' query must return false (Top-K Retrieval)"
    );

    let (bypass_localized2, _) =
        store.should_bypass_retrieval(session_id, "Give me an overview of section 2.");
    assert!(
        !bypass_localized2,
        "Localized 'overview of section 2' query must return false (Top-K Retrieval)"
    );

    let (bypass_localized3, _) =
        store.should_bypass_retrieval(session_id, "Can you summarize this chapter.");
    assert!(
        !bypass_localized3,
        "Punctuation-ending 'summarize this chapter.' must return false (Top-K Retrieval)"
    );

    let (bypass_localized4, _) = store.should_bypass_retrieval(session_id, "Summarize chapter3");
    assert!(
        !bypass_localized4,
        "No-space 'Summarize chapter3' must return false (Top-K Retrieval)"
    );

    // Assert words starting with prefix (participation, pagination, lineage) trigger broad summarization cleanly
    let (bypass_part, _) =
        store.should_bypass_retrieval(session_id, "Summarize the standard participation model");
    assert!(
        bypass_part,
        "'participation' must NOT trigger localized guard"
    );

    let (bypass_page, _) = store.should_bypass_retrieval(
        session_id,
        "Give me a summary of pagination in this architecture",
    );
    assert!(bypass_page, "'pagination' must NOT trigger localized guard");

    // Assert false positive guards: narrow targets without explicit whole-doc scope return false
    let (bypass_narrow1, _) = store.should_bypass_retrieval(
        session_id,
        "Can you outline a solution for fixing this Rust closure error?",
    );
    assert!(
        !bypass_narrow1,
        "Narrow code/error query must NOT trigger whole-document fallback"
    );

    let (bypass_narrow2, _) = store.should_bypass_retrieval(
        session_id,
        "What were the key takeaways from chapter 3 only?",
    );
    assert!(
        !bypass_narrow2,
        "Narrow chapter query must NOT trigger whole-document fallback"
    );

    Ok(())
}
