use amber_lib::ocr::{BundledOcrEngine, OcrEngine};
use std::env;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example ocr_demo -- <path_to_image>");
        println!("Example: cargo run --example ocr_demo -- C:\\Users\\name\\Desktop\\sample.png");
        return Ok(());
    }

    let img_path = Path::new(&args[1]);
    if !img_path.is_file() {
        eprintln!("Error: Image file does not exist at {}", img_path.display());
        return Ok(());
    }

    println!("Loading image from: {}", img_path.display());
    let img = image::open(img_path)?;

    println!("Initializing BundledOcrEngine (loading ONNX models from ~/.amber/models/ocr/)...");
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

    Ok(())
}
