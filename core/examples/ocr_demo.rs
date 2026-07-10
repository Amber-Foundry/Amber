use amber_lib::ingest::{ImportJobProgress, IngestJobConfig, IngestJobEngine};
use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example ocr_demo -- <path_to_image_or_pdf>");
        println!("Example: cargo run --example ocr_demo -- ./sample.pdf");
        println!("Example: cargo run --example ocr_demo -- ./sample.png");
        return Ok(());
    }

    let file_path = Path::new(&args[1]);
    if !file_path.is_file() {
        eprintln!("Error: File does not exist at {}", file_path.display());
        return Ok(());
    }

    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    let config = IngestJobConfig {
        rasterization_dpi: 300,
        target_chunk_tokens: 300,
        overlap_chunk_tokens: 50,
        ..Default::default()
    };

    println!("==================================================");
    println!("    AMBER INGESTION JOB ENGINE CLI DEMO");
    println!("==================================================");
    println!("File: {}", file_path.display());

    if ext == "pdf" {
        let (tx, rx) = mpsc::channel::<ImportJobProgress>();

        // Spawn a thread to print progress events as they happen
        let progress_handle = thread::spawn(move || {
            while let Ok(prog) = rx.recv() {
                println!(
                    "  [Job Progress] Page {}/{} - Status: {} (Digital: {}, OCR: {}, Hybrid: {}, Conf: {:.1}%)",
                    prog.current_page,
                    prog.total_pages,
                    prog.status,
                    prog.digital_pages,
                    prog.ocr_pages,
                    prog.hybrid_pages,
                    prog.avg_ocr_confidence * 100.0
                );
            }
        });

        let result =
            IngestJobEngine::process_pdf_job("job-cli-demo", file_path, config, Some(tx), None)?;

        let _ = progress_handle.join();

        let mut output_log = String::new();
        output_log.push_str("==================================================\n");
        output_log.push_str("               INGESTION JOB METRICS\n");
        output_log.push_str("==================================================\n");
        output_log.push_str(&format!("Total Pages:   {}\n", result.total_pages));
        output_log.push_str(&format!("Digital Pages: {}\n", result.digital_pages));
        output_log.push_str(&format!("OCR Pages:     {}\n", result.ocr_pages));
        output_log.push_str(&format!("Hybrid Pages:  {}\n", result.hybrid_pages));
        output_log.push_str(&format!(
            "Avg Confidence: {:.2}%\n",
            result.avg_ocr_confidence * 100.0
        ));
        output_log.push_str(&format!("Total Chunks:  {}\n", result.chunks.len()));

        output_log.push_str("\n==================================================\n");
        output_log.push_str("            GENERATED IMPORT CHUNKS\n");
        output_log.push_str("==================================================\n");
        for chunk in &result.chunks {
            output_log.push_str(&format!(
                "\n--- [Chunk #{}] (Tokens: {}, Heading: {:?}, Type: {}) ---\n",
                chunk.chunk_index, chunk.token_count, chunk.heading_context, chunk.chunk_type
            ));
            output_log.push_str(&chunk.text);
            output_log.push('\n');
        }

        output_log.push_str("\n==================================================\n");
        output_log.push_str("            EXTRACTED CANDIDATE NODES\n");
        output_log.push_str("==================================================\n");
        output_log.push_str(&format!("Total Candidates: {}\n", result.candidates.len()));
        for (i, candidate) in result.candidates.iter().enumerate() {
            output_log.push_str(&format!(
                "\n--- [Candidate #{}] Title: {} (Confidence: {:.2}) ---\n",
                i + 1,
                candidate.title,
                candidate.confidence
            ));
            output_log.push_str(&format!("Type:      {:?}\n", candidate.node_type));
            output_log.push_str(&format!("Vault Key: {:?}\n", candidate.target_vault_key));
            output_log.push_str(&format!("Summary:   {}\n", candidate.summary));
            if let Some(ref detail) = candidate.detail {
                output_log.push_str(&format!("Detail:\n{}\n", detail));
            }
            if let Some(ref meta) = candidate.meta {
                output_log.push_str(&format!(
                    "Meta JSON: {}\n",
                    serde_json::to_string_pretty(meta).unwrap_or_default()
                ));
            }
        }

        let mut file = File::create("ocr_output.txt")?;
        file.write_all(output_log.as_bytes())?;
        println!("\nIngestion completed successfully!");
        println!("Detailed metrics, chunks, and candidate nodes written to: ocr_output.txt");
    } else {
        println!("Running single image ingestion job...");
        let result = IngestJobEngine::process_image_job("job-cli-image-demo", file_path, config)?;

        let mut output_log = String::new();
        output_log.push_str("==================================================\n");
        output_log.push_str("               INGESTION JOB METRICS\n");
        output_log.push_str("==================================================\n");
        output_log.push_str(&format!("Total Pages:   {}\n", result.total_pages));
        output_log.push_str(&format!(
            "Avg Confidence: {:.2}%\n",
            result.avg_ocr_confidence * 100.0
        ));
        output_log.push_str(&format!("Total Chunks:  {}\n", result.chunks.len()));

        output_log.push_str("\n==================================================\n");
        output_log.push_str("            GENERATED IMPORT CHUNKS\n");
        output_log.push_str("==================================================\n");
        for chunk in &result.chunks {
            output_log.push_str(&format!(
                "\n--- [Chunk #{}] (Tokens: {}, Heading: {:?}, Type: {}) ---\n",
                chunk.chunk_index, chunk.token_count, chunk.heading_context, chunk.chunk_type
            ));
            output_log.push_str(&chunk.text);
            output_log.push('\n');
        }

        output_log.push_str("\n==================================================\n");
        output_log.push_str("            EXTRACTED CANDIDATE NODES\n");
        output_log.push_str("==================================================\n");
        output_log.push_str(&format!("Total Candidates: {}\n", result.candidates.len()));
        for (i, candidate) in result.candidates.iter().enumerate() {
            output_log.push_str(&format!(
                "\n--- [Candidate #{}] Title: {} (Confidence: {:.2}) ---\n",
                i + 1,
                candidate.title,
                candidate.confidence
            ));
            output_log.push_str(&format!("Type:      {:?}\n", candidate.node_type));
            output_log.push_str(&format!("Vault Key: {:?}\n", candidate.target_vault_key));
            output_log.push_str(&format!("Summary:   {}\n", candidate.summary));
            if let Some(ref detail) = candidate.detail {
                output_log.push_str(&format!("Detail:\n{}\n", detail));
            }
            if let Some(ref meta) = candidate.meta {
                output_log.push_str(&format!(
                    "Meta JSON: {}\n",
                    serde_json::to_string_pretty(meta).unwrap_or_default()
                ));
            }
        }

        let mut file = File::create("ocr_output.txt")?;
        file.write_all(output_log.as_bytes())?;
        println!("\nIngestion completed successfully!");
        println!("Detailed metrics, chunks, and candidate nodes written to: ocr_output.txt");
    }

    println!("\n==================================================");
    println!("Done!");
    Ok(())
}
