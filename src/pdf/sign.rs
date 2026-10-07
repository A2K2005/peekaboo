// Signatures as Ink annotations. The /InkList keeps the strokes for readers
// that redraw them; the appearance stream varies the width with pressure.
// FPDFAnnot_SetAP (fpdf_annot.h) uses /Rect as the stream's /BBox, so the
// stream draws in page user space.
use super::*;

impl PdfEngine {
    pub(super) fn sign(
        &self,
        page: Handle,
        rect: NormRect,
        strokes: &[Vec<[f32; 3]>],
    ) -> Result<(), String> {
        let valid = |n: &f32| n.is_finite() && (0.0..=1.0).contains(n);
        let count: usize = strokes.iter().map(Vec::len).sum();
        if count == 0
            || count > 100_000
            || !rect.iter().all(valid)
            || rect[0] >= rect[2]
            || rect[1] >= rect[3]
            || !strokes.iter().flatten().flatten().all(valid)
        {
            return Err("The signature is invalid.".into());
        }
        unsafe {
            let user = |x: f32, y: f32| -> Result<Point, String> {
                let (mut px, mut py) = (0.0, 0.0);
                if (self.api.device_to_page)(
                    page,
                    0,
                    0,
                    100_000,
                    100_000,
                    0,
                    (x * 100_000.0).round() as i32,
                    (y * 100_000.0).round() as i32,
                    &mut px,
                    &mut py,
                ) == 0
                    || !px.is_finite()
                    || !py.is_finite()
                {
                    return Err("Cannot position the signature.".into());
                }
                Ok(Point {
                    x: px as f32,
                    y: py as f32,
                })
            };
            let lines = strokes
                .iter()
                .filter(|stroke| !stroke.is_empty())
                .map(|stroke| {
                    stroke
                        .iter()
                        .map(|p| {
                            let x = rect[0] + p[0] * (rect[2] - rect[0]);
                            let y = rect[1] + p[1] * (rect[3] - rect[1]);
                            user(x, y).map(|point| (point, p[2]))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (top, bottom) = (user(rect[0], rect[1])?, user(rect[0], rect[3])?);
            let base = ((top.x - bottom.x).hypot(top.y - bottom.y) / 24.0).clamp(0.6, 3.0);
            let width = |pressure: f32| base * (0.5 + pressure);
            let pad = width(1.0);
            let all = || lines.iter().flatten().map(|(p, _)| p);
            let bounds = Rect {
                left: all().map(|p| p.x).fold(f32::INFINITY, f32::min) - pad,
                right: all().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max) + pad,
                bottom: all().map(|p| p.y).fold(f32::INFINITY, f32::min) - pad,
                top: all().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max) + pad,
            };
            let handle = (self.api.annot_create)(page, 15);
            if handle.is_null() {
                return Err("This annotation type is unavailable.".into());
            }
            let annotation = NativeHandle {
                handle,
                close: self.api.annot_close,
            };
            let check = |result: i32| {
                if result != 0 {
                    Ok(())
                } else {
                    Err("Could not add the signature.".to_string())
                }
            };
            check((self.api.annot_rect)(handle, &bounds))?;
            check((self.api.annot_flags)(handle, 4))?;
            check((self.api.annot_color)(handle, 0, 16, 33, 97, 255))?;
            check((self.api.annot_border)(handle, 0.0, 0.0, base))?;
            let mut stream = String::from("q 1 J 1 j 0.063 0.129 0.38 RG\n");
            for line in &lines {
                let points: Vec<Point> = line.iter().map(|(p, _)| *p).collect();
                if (self.api.annot_ink)(handle, points.as_ptr(), points.len()) < 0 {
                    return Err("Could not add the signature.".into());
                }
                // A one-point stroke is a zero-length segment; the round cap draws a dot.
                let segments: Vec<_> = if line.len() == 1 {
                    vec![(line[0], line[0])]
                } else {
                    line.windows(2).map(|w| (w[0], w[1])).collect()
                };
                for ((a, pa), (z, pz)) in segments {
                    stream.push_str(&format!(
                        "{:.2} w {:.2} {:.2} m {:.2} {:.2} l S\n",
                        width((pa + pz) / 2.0),
                        a.x,
                        a.y,
                        z.x,
                        z.y
                    ));
                }
            }
            stream.push('Q');
            let appearance = wide(&stream);
            if (self.api.annot_set_ap)(handle, 0, appearance.as_ptr()) == 0 {
                return self.generate_appearance(page, annotation.handle);
            }
        }
        Ok(())
    }
}
