use amber_lib::ingest::{analyze_layout, assemble_markdown, RawLayoutBlock};
use amber_lib::ocr::{
    BundledOcrEngine, OcrEngine, PdfPageType, PdfRasterizer, PdfRasterizerConfig,
};
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

            let raw_blocks = if p.page_type == PdfPageType::Digital {
                println!(
                    "[Digital Path] Extracting text & typography directly from PDF text layer..."
                );
                rasterizer.extract_digital_blocks(&pdf_bytes, i)?
            } else {
                println!("[OCR Path] Rendering page to image and running DBNet/CRNN OCR...");
                let img = rasterizer.render_page(&pdf_bytes, i, &config)?;
                let output = engine.recognize(&img)?;
                output
                    .blocks
                    .into_iter()
                    .map(|b| RawLayoutBlock::new(b.text, b.bbox).with_confidence(b.confidence))
                    .collect()
            };

            let avg_conf = if raw_blocks.is_empty() {
                1.0
            } else {
                let total: f32 = raw_blocks.iter().map(|b| b.confidence.unwrap_or(1.0)).sum();
                total / (raw_blocks.len() as f32)
            };

            println!(
                "Blocks Detected: {}, Avg Confidence: {:.2}%",
                raw_blocks.len(),
                avg_conf * 100.0
            );
            for (b_idx, block) in raw_blocks.iter().enumerate() {
                println!(
                    "  [{}] (conf: {:.1}%): \"{}\"",
                    b_idx + 1,
                    block.confidence.unwrap_or(1.0) * 100.0,
                    block.text
                );
            }

            let layout_blocks = analyze_layout(raw_blocks, p.width_pts, p.height_pts);
            let page_markdown = assemble_markdown(&layout_blocks);

            println!("\n=== ASSEMBLED MARKDOWN (Page {}) ===", i + 1);
            println!("{page_markdown}");
            println!("=====================================");
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

        println!(
            "\nBlocks Detected: {}, Avg Confidence: {:.2}%",
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

        let raw_blocks: Vec<RawLayoutBlock> = output
            .blocks
            .into_iter()
            .map(|b| RawLayoutBlock::new(b.text, b.bbox).with_confidence(b.confidence))
            .collect();

        let layout_blocks = analyze_layout(raw_blocks, img.width() as f32, img.height() as f32);
        let markdown = assemble_markdown(&layout_blocks);

        println!("\n========================================");
        println!("          OCR & MARKDOWN RESULT         ");
        println!("========================================");
        println!("{markdown}");
        println!("========================================");
    }

    Ok(())
}
