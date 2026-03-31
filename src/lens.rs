//! Lensfun XML database parser and lens profile lookup.
//! Reads camera/lens EXIF data and matches against the bundled Lensfun database
//! for distortion, vignetting, and TCA correction coefficients.

use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct LensProfile {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub distortion: Option<DistortionCoeffs>,
    pub vignetting: Option<VignetteCoeffs>,
    pub tca: Option<TcaCoeffs>,
}

#[derive(Debug, Clone, Copy)]
pub struct DistortionCoeffs {
    pub model: DistortionModel,
    pub a: f32,
    pub b: f32,
    pub c: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum DistortionModel {
    PtLens,
    Poly3,
}

#[derive(Debug, Clone, Copy)]
pub struct VignetteCoeffs {
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TcaCoeffs {
    pub vr: f32,
    pub vb: f32,
}

#[derive(Debug, Clone, Default)]
pub struct ExifInfo {
    pub camera_make: String,
    pub camera_model: String,
    pub lens_make: String,
    pub lens_model: String,
    pub focal_length: Option<f32>,
    pub aperture: Option<f32>,
}

// ---------------------------------------------------------------------------
// Lens database
// ---------------------------------------------------------------------------

pub struct LensDatabase {
    pub profiles: Vec<LensProfile>,
}

impl LensDatabase {
    /// Load the bundled Lensfun database from embedded XML files.
    pub fn load_bundled() -> Self {
        let xml_sources: &[&str] = &[include_str!("../assets/lensfun/sample-lenses.xml")];
        let mut profiles = Vec::new();
        for xml in xml_sources {
            profiles.extend(parse_lensfun_xml(xml));
        }
        Self { profiles }
    }

    /// Find a lens profile matching the given lens maker and model.
    /// Uses case-insensitive substring matching.
    pub fn find_lens(&self, maker: &str, model: &str) -> Option<&LensProfile> {
        let maker_lower = maker.to_lowercase();
        let model_lower = model.to_lowercase();
        self.profiles.iter().find(|p| {
            p.maker.to_lowercase().contains(&maker_lower)
                && p.model.to_lowercase().contains(&model_lower)
        })
    }
}

// ---------------------------------------------------------------------------
// XML parser
// ---------------------------------------------------------------------------

/// Parse a Lensfun XML database string into a list of `LensProfile`s.
pub fn parse_lensfun_xml(xml: &str) -> Vec<LensProfile> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut profiles = Vec::new();
    let mut current_lens: Option<LensProfile> = None;
    let mut in_lens = false;
    let mut in_calibration = false;
    let mut current_element = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"lens" => {
                    in_lens = true;
                    current_lens = Some(LensProfile::default());
                }
                b"calibration" if in_lens => {
                    in_calibration = true;
                }
                b"maker" | b"model" | b"mount" if in_lens => {
                    current_element = String::from_utf8_lossy(e.name().as_ref()).to_string();
                }
                _ => {}
            },
            Ok(Event::Text(e)) if in_lens => {
                if let Some(ref mut lens) = current_lens {
                    let text = e.unescape().unwrap_or_default().to_string();
                    match current_element.as_str() {
                        "maker" if lens.maker.is_empty() => lens.maker = text,
                        "model" if lens.model.is_empty() => lens.model = text,
                        "mount" if lens.mount.is_empty() => lens.mount = text,
                        _ => {}
                    }
                }
                current_element.clear();
            }
            Ok(Event::Empty(e)) if in_calibration => {
                if let Some(ref mut lens) = current_lens {
                    match e.name().as_ref() {
                        b"distortion" if lens.distortion.is_none() => {
                            lens.distortion = parse_distortion(&e);
                        }
                        b"vignetting" if lens.vignetting.is_none() => {
                            lens.vignetting = parse_vignetting(&e);
                        }
                        b"tca" if lens.tca.is_none() => {
                            lens.tca = parse_tca(&e);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"lens" => {
                    if let Some(lens) = current_lens.take() {
                        profiles.push(lens);
                    }
                    in_lens = false;
                    in_calibration = false;
                }
                b"calibration" => {
                    in_calibration = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    profiles
}

// ---------------------------------------------------------------------------
// Attribute helpers
// ---------------------------------------------------------------------------

fn attr_f32(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<f32> {
    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
        if a.key.as_ref() == name {
            String::from_utf8_lossy(&a.value).parse::<f32>().ok()
        } else {
            None
        }
    })
}

fn attr_str(e: &quick_xml::events::BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
        if a.key.as_ref() == name {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

fn parse_distortion(e: &quick_xml::events::BytesStart) -> Option<DistortionCoeffs> {
    let model_str = attr_str(e, b"model")?;
    match model_str.as_str() {
        "ptlens" => Some(DistortionCoeffs {
            model: DistortionModel::PtLens,
            a: attr_f32(e, b"a").unwrap_or(0.0),
            b: attr_f32(e, b"b").unwrap_or(0.0),
            c: attr_f32(e, b"c").unwrap_or(0.0),
        }),
        "poly3" => Some(DistortionCoeffs {
            model: DistortionModel::Poly3,
            a: attr_f32(e, b"k1").unwrap_or(0.0),
            b: 0.0,
            c: 0.0,
        }),
        _ => None,
    }
}

fn parse_vignetting(e: &quick_xml::events::BytesStart) -> Option<VignetteCoeffs> {
    Some(VignetteCoeffs {
        k1: attr_f32(e, b"k1")?,
        k2: attr_f32(e, b"k2")?,
        k3: attr_f32(e, b"k3")?,
    })
}

fn parse_tca(e: &quick_xml::events::BytesStart) -> Option<TcaCoeffs> {
    Some(TcaCoeffs {
        vr: attr_f32(e, b"vr").unwrap_or(1.0),
        vb: attr_f32(e, b"vb").unwrap_or(1.0),
    })
}

// ---------------------------------------------------------------------------
// EXIF reader
// ---------------------------------------------------------------------------

/// Read EXIF data from an image file.
pub fn read_exif(path: &Path) -> Option<ExifInfo> {
    let file = std::fs::File::open(path).ok()?;
    let exif_data = exif::Reader::new()
        .read_from_container(&mut std::io::BufReader::new(file))
        .ok()?;

    let get_str = |tag: exif::Tag| -> String {
        exif_data
            .get_field(tag, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string().trim().to_string())
            .unwrap_or_default()
    };

    let get_rational = |tag: exif::Tag| -> Option<f32> {
        let field = exif_data.get_field(tag, exif::In::PRIMARY)?;
        match &field.value {
            exif::Value::Rational(ref v) if !v.is_empty() => Some(v[0].to_f64() as f32),
            _ => None,
        }
    };

    Some(ExifInfo {
        camera_make: get_str(exif::Tag::Make),
        camera_model: get_str(exif::Tag::Model),
        lens_make: get_str(exif::Tag::LensMake),
        lens_model: get_str(exif::Tag::LensModel),
        focal_length: get_rational(exif::Tag::FocalLength),
        aperture: get_rational(exif::Tag::FNumber),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"
    <lensdatabase version="2">
        <lens>
            <maker>Sony</maker>
            <model>E 16mm f/2.8</model>
            <mount>Sony E</mount>
            <cropfactor>1.534</cropfactor>
            <calibration>
                <distortion model="ptlens" focal="16" a="0.01701" b="-0.02563" c="-0.0052"/>
                <tca model="poly3" focal="16" br="-0.0003027" vr="1.0010272" bb="0.0003454" vb="0.9993952"/>
                <vignetting model="pa" focal="16" aperture="2.8" distance="0.25" k1="-1.8891" k2="1.7993" k3="-0.7326"/>
            </calibration>
        </lens>
    </lensdatabase>
    "#;

    #[test]
    fn parse_lens_from_xml() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].maker, "Sony");
        assert_eq!(profiles[0].model, "E 16mm f/2.8");
    }

    #[test]
    fn parse_distortion_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let dist = profiles[0].distortion.unwrap();
        assert!((dist.a - 0.01701).abs() < 0.0001);
        assert!((dist.b - (-0.02563)).abs() < 0.0001);
        assert!((dist.c - (-0.0052)).abs() < 0.0001);
    }

    #[test]
    fn parse_vignetting_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let vig = profiles[0].vignetting.unwrap();
        assert!((vig.k1 - (-1.8891)).abs() < 0.0001);
        assert!((vig.k2 - 1.7993).abs() < 0.0001);
        assert!((vig.k3 - (-0.7326)).abs() < 0.0001);
    }

    #[test]
    fn parse_tca_coefficients() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let tca = profiles[0].tca.unwrap();
        assert!((tca.vr - 1.0010272).abs() < 0.0001);
        assert!((tca.vb - 0.9993952).abs() < 0.0001);
    }

    #[test]
    fn lookup_lens_by_model_substring() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let db = LensDatabase { profiles };
        let result = db.find_lens("Sony", "E 16mm f/2.8");
        assert!(result.is_some());
    }

    #[test]
    fn lookup_lens_not_found() {
        let profiles = parse_lensfun_xml(SAMPLE_XML);
        let db = LensDatabase { profiles };
        let result = db.find_lens("Nonexistent", "fake lens");
        assert!(result.is_none());
    }
}
