use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// In-memory representation of a PPTX file (path -> bytes).
pub type PptxArchive = HashMap<String, Vec<u8>>;

/// Read a PPTX file into memory as a map of internal paths to their bytes.
pub fn read_pptx(path: &Path) -> Result<PptxArchive> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("ファイルを開けません: {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("無効なPPTXファイル: {}", path.display()))?;

    let mut entries = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        entries.insert(name, buf);
    }
    Ok(entries)
}

/// Write a PPTX archive (in-memory map) to a file.
pub fn write_pptx(archive: &PptxArchive, path: &Path) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("出力ファイルを作成できません: {}", path.display()))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write [Content_Types].xml first (some readers expect it early)
    if let Some(content) = archive.get("[Content_Types].xml") {
        writer.start_file("[Content_Types].xml", options)?;
        writer.write_all(content)?;
    }

    for (name, data) in archive {
        if name == "[Content_Types].xml" {
            continue;
        }
        writer.start_file(name, options)?;
        writer.write_all(data)?;
    }

    writer.finish()?;
    Ok(())
}

/// Slide info extracted from presentation.xml
pub struct SlideInfo {
    /// Slide ID (the id attribute in <p:sldId>)
    pub id: u32,
    /// Relationship ID (e.g. "rId2")
    pub r_id: String,
}

/// Parse presentation.xml to extract the ordered list of slides.
pub fn parse_slide_list(presentation_xml: &[u8]) -> Result<Vec<SlideInfo>> {
    let mut reader = Reader::from_reader(presentation_xml);
    reader.config_mut().trim_text(true);
    let mut slides = Vec::new();
    let mut buf = Vec::new();
    let mut in_slide_id_list = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name_bytes = e.name().as_ref().to_vec();
                let local = local_name(&name_bytes);
                if local == b"sldIdLst" {
                    in_slide_id_list = true;
                } else if in_slide_id_list && local == b"sldId" {
                    let mut id = 0u32;
                    let mut r_id = String::new();
                    for attr in e.attributes().flatten() {
                        let key = attr.key.as_ref();
                        if key == b"id" {
                            id = std::str::from_utf8(&attr.value)?.parse()?;
                        } else if key == b"r:id"
                            || key.ends_with(b":id")
                        {
                            // r:id attribute
                            let val = std::str::from_utf8(&attr.value)?;
                            if val.starts_with("rId") {
                                r_id = val.to_string();
                            }
                        }
                    }
                    if !r_id.is_empty() {
                        slides.push(SlideInfo { id, r_id });
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if local_name(e.name().as_ref()) == b"sldIdLst" {
                    in_slide_id_list = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("XML解析エラー: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(slides)
}

/// Relationship entry from a .rels file.
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
}

/// Parse a .rels XML file to extract relationships.
pub fn parse_rels(rels_xml: &[u8]) -> Result<Vec<Relationship>> {
    let mut reader = Reader::from_reader(rels_xml);
    reader.config_mut().trim_text(true);
    let mut rels = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if local_name(e.name().as_ref()) == b"Relationship" {
                    let mut id = String::new();
                    let mut rel_type = String::new();
                    let mut target = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"Id" => id = std::str::from_utf8(&attr.value)?.to_string(),
                            b"Type" => rel_type = std::str::from_utf8(&attr.value)?.to_string(),
                            b"Target" => target = std::str::from_utf8(&attr.value)?.to_string(),
                            _ => {}
                        }
                    }
                    rels.push(Relationship {
                        id,
                        rel_type,
                        target,
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow::anyhow!("rels XML解析エラー: {}", e)),
            _ => {}
        }
        buf.clear();
    }
    Ok(rels)
}

/// Add a slideMaster ID entry to presentation.xml's <p:sldMasterIdLst>. Returns the modified XML.
pub fn add_master_to_presentation_xml(
    xml: &[u8],
    master_id: u32,
    r_id: &str,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(ref e)) if local_name(e.name().as_ref()) == b"sldMasterIdLst" => {
                let mut elem = BytesStart::new("p:sldMasterId");
                elem.push_attribute(("id", master_id.to_string().as_str()));
                elem.push_attribute(("r:id", r_id));
                writer.write_event(Event::Empty(elem))?;
                writer.write_event(Event::End(e.clone()))?;
            }
            Ok(Event::Eof) => {
                writer.write_event(Event::Eof)?;
                break;
            }
            Ok(e) => {
                writer.write_event(e)?;
            }
            Err(e) => return Err(anyhow::anyhow!("XML処理エラー: {}", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner())
}

/// Add a slide ID entry to presentation.xml. Returns the modified XML.
pub fn add_slide_to_presentation_xml(
    xml: &[u8],
    slide_id: u32,
    r_id: &str,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(ref e)) if local_name(e.name().as_ref()) == b"sldIdLst" => {
                // Insert new sldId before closing </p:sldIdLst>
                let mut elem = BytesStart::new("p:sldId");
                elem.push_attribute(("id", slide_id.to_string().as_str()));
                elem.push_attribute(("r:id", r_id));
                writer.write_event(Event::Empty(elem))?;
                writer.write_event(Event::End(e.clone()))?;
            }
            Ok(Event::Eof) => {
                writer.write_event(Event::Eof)?;
                break;
            }
            Ok(e) => {
                writer.write_event(e)?;
            }
            Err(e) => return Err(anyhow::anyhow!("XML処理エラー: {}", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner())
}

/// Add a relationship entry to a .rels file. Returns the modified XML.
pub fn add_relationship_to_rels(
    xml: &[u8],
    id: &str,
    rel_type: &str,
    target: &str,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(ref e)) if local_name(e.name().as_ref()) == b"Relationships" => {
                // Insert new Relationship before closing </Relationships>
                let mut elem = BytesStart::new("Relationship");
                elem.push_attribute(("Id", id));
                elem.push_attribute(("Type", rel_type));
                elem.push_attribute(("Target", target));
                writer.write_event(Event::Empty(elem))?;
                writer.write_event(Event::End(e.clone()))?;
            }
            Ok(Event::Eof) => {
                writer.write_event(Event::Eof)?;
                break;
            }
            Ok(e) => {
                writer.write_event(e)?;
            }
            Err(e) => return Err(anyhow::anyhow!("rels XML処理エラー: {}", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner())
}

/// Add a content type override to [Content_Types].xml. Returns the modified XML.
pub fn add_content_type_override(
    xml: &[u8],
    part_name: &str,
    content_type: &str,
) -> Result<Vec<u8>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::End(ref e)) if local_name(e.name().as_ref()) == b"Types" => {
                let mut elem = BytesStart::new("Override");
                elem.push_attribute(("PartName", part_name));
                elem.push_attribute(("ContentType", content_type));
                writer.write_event(Event::Empty(elem))?;
                writer.write_event(Event::End(e.clone()))?;
            }
            Ok(Event::Eof) => {
                writer.write_event(Event::Eof)?;
                break;
            }
            Ok(e) => {
                writer.write_event(e)?;
            }
            Err(e) => return Err(anyhow::anyhow!("Content_Types XML処理エラー: {}", e)),
        }
        buf.clear();
    }
    Ok(writer.into_inner())
}

/// Get the highest rId number from a rels file.
pub fn max_rid(rels: &[Relationship]) -> u32 {
    rels.iter()
        .filter_map(|r| r.id.strip_prefix("rId").and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0)
}

/// Get the highest slide ID from the slide list.
pub fn max_slide_id(slides: &[SlideInfo]) -> u32 {
    slides.iter().map(|s| s.id).max().unwrap_or(255)
}

/// Extract local name from a possibly namespaced XML name (e.g. "p:sldId" -> "sldId").
fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}
