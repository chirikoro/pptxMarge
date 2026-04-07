use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::pptx;

const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const SLIDE_CT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";

/// Merge multiple PPTX files into one output file.
/// Simple strategy: keep base file's masters/layouts/themes,
/// only copy slides and their direct resources (images etc).
pub fn merge_pptx_files(input_files: &[impl AsRef<Path>], output: &Path) -> Result<()> {
    if input_files.len() < 2 {
        anyhow::bail!("結合するには2つ以上のファイルが必要です");
    }

    let base_path = input_files[0].as_ref();
    let mut archive = pptx::read_pptx(base_path)
        .with_context(|| format!("ベースファイルの読み込み失敗: {}", base_path.display()))?;

    let pres_xml = archive
        .get("ppt/presentation.xml")
        .context("ppt/presentation.xmlがありません")?
        .clone();
    let base_slides = pptx::parse_slide_list(&pres_xml)?;
    let mut next_slide_id = pptx::max_slide_id(&base_slides) + 1;

    let pres_rels_xml = archive
        .get("ppt/_rels/presentation.xml.rels")
        .context("presentation.xml.relsがありません")?
        .clone();
    let base_rels = pptx::parse_rels(&pres_rels_xml)?;
    let mut next_rid = pptx::max_rid(&base_rels) + 1;

    let mut next_slide_num = find_max_number(&archive, "ppt/slides/", "slide", ".xml") + 1;
    let mut next_media_num = find_max_media_number(&archive) + 1;

    // Find the base file's first slide layout target (for imported slides)
    let base_layout_target = find_base_layout_target(&archive);

    // Merge Default extension entries from all source files
    for file_path in &input_files[1..] {
        let file_path = file_path.as_ref();
        let src = pptx::read_pptx(file_path)
            .with_context(|| format!("ファイルの読み込み失敗: {}", file_path.display()))?;

        if let Some(ct_xml) = src.get("[Content_Types].xml") {
            let (_, defaults) = pptx::parse_content_types(ct_xml)?;
            for (ext, ct) in &defaults {
                let dest_ct = archive.get("[Content_Types].xml").unwrap().clone();
                let updated = pptx::add_content_type_default(&dest_ct, ext, ct)?;
                archive.insert("[Content_Types].xml".to_string(), updated);
            }
        }
    }

    // Process each additional file
    for file_path in &input_files[1..] {
        let file_path = file_path.as_ref();
        let src = pptx::read_pptx(file_path)
            .with_context(|| format!("ファイルの読み込み失敗: {}", file_path.display()))?;

        let src_pres_xml = src
            .get("ppt/presentation.xml")
            .with_context(|| format!("{}にpresentation.xmlがありません", file_path.display()))?;
        let src_slides = pptx::parse_slide_list(src_pres_xml)?;

        let src_pres_rels_xml = src
            .get("ppt/_rels/presentation.xml.rels")
            .with_context(|| format!("{}にpresentation.xml.relsがありません", file_path.display()))?;
        let src_rels = pptx::parse_rels(src_pres_rels_xml)?;
        let rid_to_target: HashMap<String, String> = src_rels
            .iter()
            .map(|r| (r.id.clone(), r.target.clone()))
            .collect();

        for slide_info in &src_slides {
            let src_slide_rel_target = match rid_to_target.get(&slide_info.r_id) {
                Some(t) => t,
                None => continue,
            };

            let src_slide_path = normalize_ppt_path(src_slide_rel_target, "ppt");

            let slide_xml = match src.get(&src_slide_path) {
                Some(data) => data.clone(),
                None => continue,
            };

            let new_slide_path = format!("ppt/slides/slide{}.xml", next_slide_num);
            let new_slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", next_slide_num);

            // Copy slide XML as-is
            archive.insert(new_slide_path.clone(), slide_xml);

            // Build new .rels for this slide: copy media/resources, use base layout
            let src_slide_filename = src_slide_path.rsplit('/').next().unwrap_or("slide1.xml");
            let src_slide_rels_path = format!("ppt/slides/_rels/{}.rels", src_slide_filename);

            let new_rels = if let Some(rels_data) = src.get(&src_slide_rels_path) {
                let rels = pptx::parse_rels(rels_data)?;
                let mut entries: Vec<RelEntry> = Vec::new();

                for rel in &rels {
                    if rel.rel_type.ends_with("/slideLayout") {
                        // Use base file's layout instead
                        entries.push(RelEntry {
                            id: rel.id.clone(),
                            rel_type: rel.rel_type.clone(),
                            target: base_layout_target.clone(),
                            target_mode: None,
                        });
                    } else if rel.target_mode.as_deref() == Some("External")
                        || rel.target.starts_with("http://")
                        || rel.target.starts_with("https://")
                    {
                        // External link - keep as-is
                        entries.push(RelEntry {
                            id: rel.id.clone(),
                            rel_type: rel.rel_type.clone(),
                            target: rel.target.clone(),
                            target_mode: rel.target_mode.clone(),
                        });
                    } else {
                        // Media/resource - copy with new name
                        let src_abs = resolve_path("ppt/slides", &rel.target);
                        if let Some(data) = src.get(&src_abs) {
                            let ext = src_abs.rsplit('.').next().unwrap_or("bin");
                            let new_media = format!("ppt/media/media{}.{}", next_media_num, ext);
                            let new_rel_target = format!("../media/media{}.{}", next_media_num, ext);
                            next_media_num += 1;

                            archive.insert(new_media, data.clone());
                            entries.push(RelEntry {
                                id: rel.id.clone(),
                                rel_type: rel.rel_type.clone(),
                                target: new_rel_target,
                                target_mode: None,
                            });
                        } else {
                            // Resource not found, keep original reference
                            entries.push(RelEntry {
                                id: rel.id.clone(),
                                rel_type: rel.rel_type.clone(),
                                target: rel.target.clone(),
                                target_mode: rel.target_mode.clone(),
                            });
                        }
                    }
                }
                build_rels_xml(&entries)?
            } else {
                // No .rels file - create minimal one with just layout reference
                build_rels_xml(&[RelEntry {
                    id: "rId1".to_string(),
                    rel_type: "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout".to_string(),
                    target: base_layout_target.clone(),
                    target_mode: None,
                }])?
            };

            archive.insert(new_slide_rels_path, new_rels);

            // Add slide to presentation.xml
            let pres = archive.get("ppt/presentation.xml").unwrap().clone();
            let rid_str = format!("rId{}", next_rid);
            let updated_pres = pptx::add_slide_to_presentation_xml(&pres, next_slide_id, &rid_str)?;
            archive.insert("ppt/presentation.xml".to_string(), updated_pres);

            // Add relationship in presentation.xml.rels
            let rels = archive.get("ppt/_rels/presentation.xml.rels").unwrap().clone();
            let slide_target = format!("slides/slide{}.xml", next_slide_num);
            let updated_rels = pptx::add_relationship_to_rels(&rels, &rid_str, SLIDE_REL_TYPE, &slide_target)?;
            archive.insert("ppt/_rels/presentation.xml.rels".to_string(), updated_rels);

            // Add content type
            let ct = archive.get("[Content_Types].xml").unwrap().clone();
            let part_name = format!("/ppt/slides/slide{}.xml", next_slide_num);
            let updated_ct = pptx::add_content_type_override(&ct, &part_name, SLIDE_CT)?;
            archive.insert("[Content_Types].xml".to_string(), updated_ct);

            next_slide_id += 1;
            next_rid += 1;
            next_slide_num += 1;
        }
    }

    // Update docProps/app.xml
    update_app_xml_slide_count(&mut archive);

    pptx::write_pptx(&archive, output)?;
    Ok(())
}

struct RelEntry {
    id: String,
    rel_type: String,
    target: String,
    target_mode: Option<String>,
}

fn build_rels_xml(entries: &[RelEntry]) -> Result<Vec<u8>> {
    use quick_xml::events::{BytesDecl, BytesStart, Event};
    use quick_xml::writer::Writer;

    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), Some("yes"))))?;

    let mut root = BytesStart::new("Relationships");
    root.push_attribute(("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships"));
    writer.write_event(Event::Start(root))?;

    for entry in entries {
        let mut elem = BytesStart::new("Relationship");
        elem.push_attribute(("Id", entry.id.as_str()));
        elem.push_attribute(("Type", entry.rel_type.as_str()));
        elem.push_attribute(("Target", entry.target.as_str()));
        if let Some(mode) = &entry.target_mode {
            elem.push_attribute(("TargetMode", mode.as_str()));
        }
        writer.write_event(Event::Empty(elem))?;
    }

    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Relationships")))?;
    Ok(writer.into_inner())
}

/// Find the base file's first slide's layout target.
fn find_base_layout_target(archive: &pptx::PptxArchive) -> String {
    // Check slide1, slide2, etc.
    for i in 1..=20 {
        let rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", i);
        if let Some(rels_data) = archive.get(&rels_path) {
            if let Ok(rels) = pptx::parse_rels(rels_data) {
                for rel in &rels {
                    if rel.rel_type.ends_with("/slideLayout") {
                        return rel.target.clone();
                    }
                }
            }
        }
    }
    "../slideLayouts/slideLayout1.xml".to_string()
}

fn normalize_ppt_path(rel_target: &str, base: &str) -> String {
    if rel_target.starts_with("ppt/") || rel_target.starts_with("/ppt/") {
        rel_target.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", base, rel_target)
    }
}

fn resolve_path(base_dir: &str, relative: &str) -> String {
    if relative.starts_with('/') {
        return relative.trim_start_matches('/').to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').collect();
    for segment in relative.split('/') {
        match segment {
            ".." => { parts.pop(); }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn find_max_number(archive: &pptx::PptxArchive, dir: &str, prefix: &str, suffix: &str) -> u32 {
    archive
        .keys()
        .filter_map(|name| {
            if !name.starts_with(dir) || name.contains("/_rels/") {
                return None;
            }
            let fname = name.rsplit('/').next()?;
            let num_str = fname.strip_prefix(prefix)?.strip_suffix(suffix)?;
            num_str.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

fn find_max_media_number(archive: &pptx::PptxArchive) -> u32 {
    archive
        .keys()
        .filter_map(|name| {
            if !name.starts_with("ppt/media/") {
                return None;
            }
            let fname = name.rsplit('/').next()?;
            let without_ext = fname.rsplit('.').last()?;
            let num_str = without_ext.trim_start_matches(|c: char| c.is_alphabetic());
            num_str.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// Update slide count in docProps/app.xml.
fn update_app_xml_slide_count(archive: &mut pptx::PptxArchive) {
    let total_slides = archive.keys()
        .filter(|k| k.starts_with("ppt/slides/") && k.ends_with(".xml") && !k.contains("/_rels/"))
        .count();

    if let Some(app_xml) = archive.get("docProps/app.xml").cloned() {
        let xml_str = String::from_utf8_lossy(&app_xml).to_string();
        if let Some(start) = xml_str.find("<Slides>") {
            if let Some(end) = xml_str[start..].find("</Slides>") {
                let mut result = String::new();
                result.push_str(&xml_str[..start]);
                result.push_str(&format!("<Slides>{}</Slides>", total_slides));
                result.push_str(&xml_str[start + end + "</Slides>".len()..]);
                archive.insert("docProps/app.xml".to_string(), result.into_bytes());
            }
        }
    }
}
