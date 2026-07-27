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
    let embeddings = amber_lib::ephemeral::compute_ephemeral_chunk_embeddings(&specs);
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
        embeddings_computed: true,
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
