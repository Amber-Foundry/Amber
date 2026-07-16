/// Soft template envelope used to validate empirical layout detection (warnings only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutTemplate {
    ApaSingleColumn,
    IeeeTwoColumn,
    UsenixTwoColumn,
    BusinessSingleColumn,
}

impl LayoutTemplate {
    fn hint_name(self) -> &'static str {
        match self {
            Self::ApaSingleColumn => "apa_single_column",
            Self::IeeeTwoColumn => "ieee_two_column",
            Self::UsenixTwoColumn => "usenix_two_column",
            Self::BusinessSingleColumn => "business_single_column",
        }
    }
}

/// Result of cross-checking empirical layout against known document envelopes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemplateMatch {
    pub layout_hint: Option<String>,
    pub confidence: Option<f32>,
    pub warnings: Vec<String>,
}

#[allow(dead_code)]
struct Envelope {
    template: LayoutTemplate,
    #[allow(dead_code)]
    side_margin_ratio: f32,
    columns: u8,
    gutter_min_ratio: f32,
    gutter_max_ratio: f32,
    #[allow(dead_code)]
    first_page_title_zone_ratio: f32,
}

const ENVELOPES: &[Envelope] = &[
    Envelope {
        template: LayoutTemplate::ApaSingleColumn,
        side_margin_ratio: 0.125,
        columns: 1,
        gutter_min_ratio: 0.0,
        gutter_max_ratio: 0.0,
        first_page_title_zone_ratio: 0.0,
    },
    Envelope {
        template: LayoutTemplate::BusinessSingleColumn,
        side_margin_ratio: 0.125,
        columns: 1,
        gutter_min_ratio: 0.0,
        gutter_max_ratio: 0.0,
        first_page_title_zone_ratio: 0.0,
    },
    Envelope {
        template: LayoutTemplate::IeeeTwoColumn,
        side_margin_ratio: 0.10,
        columns: 2,
        gutter_min_ratio: 0.45,
        gutter_max_ratio: 0.55,
        first_page_title_zone_ratio: 0.15,
    },
    Envelope {
        template: LayoutTemplate::UsenixTwoColumn,
        side_margin_ratio: 0.11,
        columns: 2,
        gutter_min_ratio: 0.45,
        gutter_max_ratio: 0.55,
        first_page_title_zone_ratio: 0.15,
    },
];

/// Compares detected column structure against standard layout envelopes.
pub fn evaluate_template_match(
    page_width: f32,
    band_count: usize,
    column_splits: usize,
    detected_gutter_x: Option<f32>,
) -> TemplateMatch {
    if page_width <= 0.0 {
        return TemplateMatch::default();
    }

    let two_column = column_splits > 0;
    let candidates: Vec<_> = ENVELOPES
        .iter()
        .filter(|env| {
            if two_column {
                env.columns == 2
            } else {
                env.columns == 1
            }
        })
        .collect();

    if candidates.is_empty() {
        return TemplateMatch::default();
    }

    let mut best: Option<(&Envelope, f32)> = None;
    for env in candidates {
        let mut score = 0.7f32;
        if env.columns == 2 {
            if let Some(gutter_x) = detected_gutter_x {
                let gutter_ratio = gutter_x / page_width;
                if gutter_ratio >= env.gutter_min_ratio && gutter_ratio <= env.gutter_max_ratio {
                    score += 0.25;
                } else {
                    score -= 0.15;
                }
            } else {
                score -= 0.1;
            }
        } else if column_splits == 0 {
            score += 0.2;
        }

        if band_count >= 1 {
            score += 0.05;
        }

        match best {
            Some((_, best_score)) if score <= best_score => {}
            _ => best = Some((env, score.clamp(0.0, 1.0))),
        }
    }

    let Some((envelope, confidence)) = best else {
        return TemplateMatch::default();
    };

    let mut warnings = Vec::new();
    if envelope.columns == 2 {
        if let Some(gutter_x) = detected_gutter_x {
            let gutter_ratio = gutter_x / page_width;
            if gutter_ratio < envelope.gutter_min_ratio || gutter_ratio > envelope.gutter_max_ratio
            {
                warnings.push("gutter_outside_two_column_envelope".to_string());
            }
        } else if column_splits > 0 {
            warnings.push("column_split_without_gutter".to_string());
        }
    }

    TemplateMatch {
        layout_hint: Some(envelope.template.hint_name().to_string()),
        confidence: Some(confidence),
        warnings,
    }
}

/// Heuristic for slide/deck or decorative layouts that resist column-band flow.
pub fn detect_non_flow_layout(
    snapshot: &crate::ingest::layout::LayoutDebugSnapshot,
    block_count: usize,
    _page_width: f32,
    _page_height: f32,
) -> Option<String> {
    if snapshot.band_count == 0 {
        return None;
    }
    if snapshot.column_splits > snapshot.band_count {
        return Some("non_flow_document".to_string());
    }
    if block_count < 8 && snapshot.column_splits > 2 {
        return Some("non_flow_document".to_string());
    }
    if snapshot.band_count > 12 && block_count < snapshot.band_count * 2 {
        return Some("non_flow_document".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_column_fixture_matches_ieee_family() {
        let result = evaluate_template_match(612.0, 2, 1, Some(306.0));
        assert!(
            result
                .layout_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("two_column")),
            "expected two-column hint, got {:?}",
            result.layout_hint
        );
        assert!(
            result.confidence.unwrap_or(0.0) > 0.5,
            "expected reasonable confidence"
        );
    }

    #[test]
    fn single_column_matches_apa_or_business() {
        let result = evaluate_template_match(612.0, 1, 0, None);
        assert!(
            result
                .layout_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("single_column")),
            "expected single-column hint, got {:?}",
            result.layout_hint
        );
        assert!(result.warnings.is_empty());
    }
}
