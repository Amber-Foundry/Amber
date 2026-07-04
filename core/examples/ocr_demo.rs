use amber_lib::ocr::{BundledOcrEngine, OcrEngine, PdfRasterizer, PdfRasterizerConfig};
use std::env;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example ocr_demo -- <path_to_image_or_pdf>");
        println!("Example: cargo run --example ocr_demo -- C:\\Users\\name\\Desktop\\sample.png");
        println!("Example: cargo run --example ocr_demo -- C:\\Users\\name\\Desktop\\sample.pdf");
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

    if ext == "pdf" {
        println!("Loading PDF from: {}", file_path.display());
        let rasterizer = match PdfRasterizer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Could not initialize PdfRasterizer: {e}");
                eprintln!("To test PDF rasterization, place pdfium.dll / libpdfium in ~/.amber/resources/pdfium/");
                return Ok(());
            }
        };

        let pages = rasterizer.scan_file(file_path)?;
        println!("PDF Scanned Successfully: Total Pages = {}", pages.len());
        for p in &pages {
            println!(
                "  Page [{}]: type = {}, size = {:.0}x{:.0} pts",
                p.page_index + 1,
                p.page_type,
                p.width_pts,
                p.height_pts
            );
        }

        println!("\nInitializing BundledOcrEngine...");
        let engine = BundledOcrEngine::new()?;
        let config = PdfRasterizerConfig::default();
        let pdf_bytes = std::fs::read(file_path)?;

        for (i, p) in pages.iter().enumerate() {
            println!(
                "\n--- Processing Page {}/{} ({}) ---",
                i + 1,
                pages.len(),
                p.page_type
            );
            let img = rasterizer.render_page(&pdf_bytes, i, &config)?;
            let output = engine.recognize(&img)?;

            println!(
                "Blocks Detected: {}, Avg Confidence: {:.2}%",
                output.blocks.len(),
                output.avg_confidence * 100.0
            );
            for (b_idx, block) in output.blocks.iter().enumerate() {
                println!(
                    "  [{}] (conf: {:.1}%): \"{}\"",
                    b_idx + 1,
                    block.confidence * 100.0,
                    block.text
                );
            }
        }
    } else {
        println!("Loading image from: {}", file_path.display());
        let img = image::open(file_path)?;

        println!(
            "Initializing BundledOcrEngine (loading ONNX models from ~/.amber/models/ocr/)..."
        );
        let engine = BundledOcrEngine::new()?;

        println!("Running OCR text detection and recognition...");
        let output = engine.recognize(&img)?;

        println!("\n========================================");
        println!("          OCR EXECUTION RESULTS         ");
        println!("========================================");
        println!("Total Text Blocks Detected: {}", output.blocks.len());
        println!(
            "Overall Average Confidence: {:.2}%",
            output.avg_confidence * 100.0
        );
        println!("----------------------------------------");

        if output.blocks.is_empty() {
            println!("No text blocks detected in this image.");
        } else {
            for (i, block) in output.blocks.iter().enumerate() {
                println!(
                    "Block [{}] (conf: {:.1}%, bbox: x={:.0}, y={:.0}, w={:.0}, h={:.0}):",
                    i + 1,
                    block.confidence * 100.0,
                    block.bbox.x,
                    block.bbox.y,
                    block.bbox.width,
                    block.bbox.height
                );
                println!("  \"{}\"", block.text);
                println!("----------------------------------------");
            }
        }
    }

    Ok(())
}
