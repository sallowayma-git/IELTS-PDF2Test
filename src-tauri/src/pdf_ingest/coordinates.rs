use super::{clean, Bounds};
use pdf_extract::{Document, Object, ObjectId};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub(crate) struct PdfPageGeometry {
    pub page_index: u32,
    pub media_box: Bounds,
    pub crop_box: Bounds,
    pub rotation: u16,
    pub user_unit: f64,
    pub display_width: f64,
    pub display_height: f64,
    pub pdf_to_display: [f64; 6],
}

impl PdfPageGeometry {
    pub(crate) fn fallback(page_index: u32, width: f64, height: f64, rotation: u16) -> Self {
        Self::new(
            page_index,
            Bounds {
                x: 0.0,
                y: 0.0,
                width: width.max(1.0),
                height: height.max(1.0),
            },
            None,
            rotation,
            1.0,
        )
    }

    fn new(
        page_index: u32,
        media_box: Bounds,
        crop_box: Option<Bounds>,
        rotation: u16,
        user_unit: f64,
    ) -> Self {
        let crop_box = crop_box.unwrap_or(media_box);
        let user_unit = if user_unit.is_finite() && user_unit > 0.0 {
            user_unit
        } else {
            1.0
        };
        let width = crop_box.width * user_unit;
        let height = crop_box.height * user_unit;
        let left = crop_box.x;
        let bottom = crop_box.y;
        let right = crop_box.right();
        let top = crop_box.bottom();
        let pdf_to_display = match rotation {
            90 => [
                0.0,
                user_unit,
                user_unit,
                0.0,
                -bottom * user_unit,
                -left * user_unit,
            ],
            180 => [
                -user_unit,
                0.0,
                0.0,
                user_unit,
                right * user_unit,
                -bottom * user_unit,
            ],
            270 => [
                0.0,
                -user_unit,
                -user_unit,
                0.0,
                top * user_unit,
                right * user_unit,
            ],
            _ => [
                user_unit,
                0.0,
                0.0,
                -user_unit,
                -left * user_unit,
                top * user_unit,
            ],
        };
        let (display_width, display_height) = if matches!(rotation, 90 | 270) {
            (height, width)
        } else {
            (width, height)
        };
        Self {
            page_index,
            media_box,
            crop_box,
            rotation,
            user_unit,
            display_width: display_width.max(1.0),
            display_height: display_height.max(1.0),
            pdf_to_display,
        }
    }

    pub(crate) fn display_point(&self, point: (f64, f64)) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.pdf_to_display;
        (a * point.0 + c * point.1 + e, b * point.0 + d * point.1 + f)
    }

    pub(crate) fn display_bounds(&self, native: Bounds) -> Bounds {
        let points = [
            (native.x, native.y),
            (native.right(), native.y),
            (native.right(), native.bottom()),
            (native.x, native.bottom()),
        ]
        .map(|point| self.display_point(point));
        bounds_for_points(&points).unwrap_or(Bounds {
            x: 0.0,
            y: 0.0,
            width: 0.01,
            height: 0.01,
        })
    }

    pub(crate) fn native_rect(&self, bounds: Bounds) -> Value {
        json!({
            "x": clean(bounds.x),
            "y": clean(bounds.y),
            "width": clean(bounds.width.max(0.01)),
            "height": clean(bounds.height.max(0.01)),
            "unit": "pt",
            "origin": "bottom-left",
            "pageRotation": self.rotation
        })
    }

    pub(crate) fn display_rect(&self, bounds: Bounds) -> Value {
        let normalized = [
            bounds.x / self.display_width,
            bounds.y / self.display_height,
            bounds.width / self.display_width,
            bounds.height / self.display_height,
        ]
        .map(|value| clean(value.clamp(0.0, 1.0)));
        json!({
            "x": clean(bounds.x),
            "y": clean(bounds.y),
            "width": clean(bounds.width.max(0.01)),
            "height": clean(bounds.height.max(0.01)),
            "unit": "pt",
            "origin": "top-left",
            "pageRotation": self.rotation,
            "normalized": normalized
        })
    }

    pub(crate) fn page_transform_value(&self) -> Value {
        json!({
            "userUnit": clean(self.user_unit),
            "pdfToDisplay": self.pdf_to_display.map(clean),
            "displayToNormalized": [
                clean(1.0 / self.display_width), 0.0, 0.0,
                clean(1.0 / self.display_height), 0.0, 0.0
            ],
            "displayWidthPt": clean(self.display_width),
            "displayHeightPt": clean(self.display_height)
        })
    }
}

fn dereference<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(reference) => document.get_object(*reference).ok(),
        _ => Some(object),
    }
}

fn inherited_object<'a>(
    document: &'a Document,
    page_id: ObjectId,
    key: &[u8],
) -> Option<&'a Object> {
    let mut current = page_id;
    for _ in 0..32 {
        let dictionary = document.get_dictionary(current).ok()?;
        if let Ok(value) = dictionary.get(key) {
            return dereference(document, value);
        }
        current = dictionary.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn inherited_number(document: &Document, page_id: ObjectId, key: &[u8]) -> Option<f64> {
    inherited_object(document, page_id, key).and_then(number)
}

fn inherited_rect(document: &Document, page_id: ObjectId, key: &[u8]) -> Option<Bounds> {
    let values = inherited_object(document, page_id, key)?.as_array().ok()?;
    if values.len() < 4 {
        return None;
    }
    let x1 = number(dereference(document, &values[0])?)?;
    let y1 = number(dereference(document, &values[1])?)?;
    let x2 = number(dereference(document, &values[2])?)?;
    let y2 = number(dereference(document, &values[3])?)?;
    Some(Bounds {
        x: x1.min(x2),
        y: y1.min(y2),
        width: (x2 - x1).abs().max(0.01),
        height: (y2 - y1).abs().max(0.01),
    })
}

fn inherited_rotation(document: &Document, page_id: ObjectId) -> u16 {
    let rotation = inherited_number(document, page_id, b"Rotate")
        .unwrap_or(0.0)
        .round() as i64;
    match rotation.rem_euclid(360) {
        90 | 180 | 270 => rotation.rem_euclid(360) as u16,
        _ => 0,
    }
}

pub(crate) fn collect_page_geometries(document: &Document) -> BTreeMap<u32, PdfPageGeometry> {
    document
        .get_pages()
        .into_iter()
        .filter_map(|(page_num, page_id)| {
            let media_box = inherited_rect(document, page_id, b"MediaBox")?;
            let crop_box = inherited_rect(document, page_id, b"CropBox");
            let rotation = inherited_rotation(document, page_id);
            let user_unit = inherited_number(document, page_id, b"UserUnit").unwrap_or(1.0);
            Some((
                page_num,
                PdfPageGeometry::new(
                    page_num.saturating_sub(1),
                    media_box,
                    crop_box,
                    rotation,
                    user_unit,
                ),
            ))
        })
        .collect()
}

pub(crate) fn bounds_for_points(points: &[(f64, f64)]) -> Option<Bounds> {
    let left = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let right = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let top = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    (left.is_finite() && bottom.is_finite() && right > left && top > bottom).then_some(Bounds {
        x: left,
        y: bottom,
        width: right - left,
        height: top - bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matrices_map_crop_box_to_upright_display() {
        let media = Bounds {
            x: -10.0,
            y: 20.0,
            width: 200.0,
            height: 100.0,
        };
        for (rotation, expected) in [
            (0, (400.0, 200.0)),
            (90, (200.0, 400.0)),
            (180, (400.0, 200.0)),
            (270, (200.0, 400.0)),
        ] {
            let geometry = PdfPageGeometry::new(0, media, None, rotation, 2.0);
            assert_eq!((geometry.display_width, geometry.display_height), expected);
            let display = geometry.display_bounds(media);
            assert!(
                (display.x).abs() < 0.001,
                "rotation={rotation}: {display:?}"
            );
            assert!(
                (display.y).abs() < 0.001,
                "rotation={rotation}: {display:?}"
            );
            assert!((display.width - expected.0).abs() < 0.001);
            assert!((display.height - expected.1).abs() < 0.001);
        }
    }
}
