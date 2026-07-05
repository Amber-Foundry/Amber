use amber_lib::ingest::{ImportJobProgress, IngestJobConfig, IngestJobEngine};
use std::env;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example ocr_demo -- <path_to_image_or_pdf>");
        println!("Example: cargo run --example ocr_demo -- C:\\Users\\name\\Desktop\\sample.pdf");
        println!("Example: cargo run --example ocr_demo -- C:\\Users\\name\\Desktop\\sample.png");
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

        let result = IngestJobEngine::process_pdf_job("job-cli-demo", file_path, config, Some(tx))?;

        let _ = progress_handle.join();

        println!("\n==================================================");
        println!("               INGESTION JOB METRICS");
        println!("==================================================");
        println!("Total Pages:   {}", result.total_pages);
        println!("Digital Pages: {}", result.digital_pages);
        println!("OCR Pages:     {}", result.ocr_pages);
        println!("Hybrid Pages:  {}", result.hybrid_pages);
        println!("Avg Confidence: {:.2}%", result.avg_ocr_confidence * 100.0);
        println!("Total Chunks:  {}", result.chunks.len());

        println!("\n==================================================");
        println!("            GENERATED IMPORT CHUNKS");
        println!("==================================================");
        for chunk in &result.chunks {
            println!(
                "\n--- [Chunk #{}] (Tokens: {}, Heading: {:?}, Type: {}) ---",
                chunk.chunk_index, chunk.token_count, chunk.heading_context, chunk.chunk_type
            );
            println!("{}", chunk.text);
        }
    } else {
        println!("Running single image ingestion job...");
        let result = IngestJobEngine::process_image_job("job-cli-image-demo", file_path, config)?;

        println!("\n==================================================");
        println!("               INGESTION JOB METRICS");
        println!("==================================================");
        println!("Total Pages:   {}", result.total_pages);
        println!("Avg Confidence: {:.2}%", result.avg_ocr_confidence * 100.0);
        println!("Total Chunks:  {}", result.chunks.len());

        println!("\n==================================================");
        println!("            GENERATED IMPORT CHUNKS");
        println!("==================================================");
        for chunk in &result.chunks {
            println!(
                "\n--- [Chunk #{}] (Tokens: {}, Heading: {:?}, Type: {}) ---",
                chunk.chunk_index, chunk.token_count, chunk.heading_context, chunk.chunk_type
            );
            println!("{}", chunk.text);
        }
    }

    println!("\n==================================================");
    println!("Done!");
    Ok(())
}
