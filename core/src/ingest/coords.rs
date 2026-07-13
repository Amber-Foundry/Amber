use crate::ingest::layout::RawLayoutBlock;
use crate::ocr::engine::Rect;

/// Scales OCR/image-space bounding boxes into PDF points (top-left y).
pub fn normalize_blocks_to_pdf_points(
    blocks: Vec<RawLayoutBlock>,
    image_width: f32,
    image_height: f32,
    width_pts: f32,
    height_pts: f32,
) -> Vec<RawLayoutBlock> {
    if blocks.is_empty() {
        return blocks;
    }
    let scale_x = width_pts / image_width.max(1.0);
    let scale_y = height_pts / image_height.max(1.0);
    blocks
        .into_iter()
        .map(|mut block| {
            block.bbox = Rect::new(
                block.bbox.x * scale_x,
                block.bbox.y * scale_y,
                block.bbox.width * scale_x,
                block.bbox.height * scale_y,
            );
            if let Some(font_size) = block.font_size {
                block.font_size = Some(font_size * scale_y);
            }
            block
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_bbox_maps_to_pdf_points() {
        let blocks = vec![RawLayoutBlock::new(
            "sample",
            Rect::new(0.0, 0.0, 100.0, 20.0),
        )];
        let normalized = normalize_blocks_to_pdf_points(blocks, 2550.0, 3300.0, 612.0, 792.0);
        let bbox = &normalized[0].bbox;
        assert!((bbox.width - 24.0).abs() < 0.01);
        assert!((bbox.height - 4.8).abs() < 0.01);
    }
}
