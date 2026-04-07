use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::pptx;

const SLIDE_CT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const LAYOUT_CT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
const MASTER_CT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml";
const THEME_CT: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.theme+xml";
const NOTES_SLIDE_CT: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
const CHART_CT: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const MASTER_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";

/// Merge multiple PPTX files into one output file.
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
    let mut next_master_id = parse_max_master_id(&pres_xml) + 1;

    let pres_rels_xml = archive
        .get("ppt/_rels/presentation.xml.rels")
        .context("presentation.xml.relsがありません")?
        .clone();
    let base_rels = pptx::parse_rels(&pres_rels_xml)?;
    let mut next_rid = pptx::max_rid(&base_rels) + 1;

    let mut counters = Counters {
        slide: find_max_number(&archive, "ppt/slides/", "slide", ".xml") + 1,
        layout: find_max_number(&archive, "ppt/slideLayouts/", "slideLayout", ".xml") + 1,
        master: find_max_number(&archive, "ppt/slideMasters/", "slideMaster", ".xml") + 1,
        theme: find_max_number(&archive, "ppt/theme/", "theme", ".xml") + 1,
        media: find_max_media_number(&archive) + 1,
        notes: find_max_number(&archive, "ppt/notesSlides/", "notesSlide", ".xml") + 1,
        chart: find_max_number(&archive, "ppt/charts/", "chart", ".xml") + 1,
    };

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

        // === Phase 1: Build COMPLETE path_remap before copying anything ===
        let mut path_remap: HashMap<String, String> = HashMap::new();

        // Assign new names for themes
        let src_themes: Vec<String> = sorted_keys(&src, "ppt/theme/", ".xml");
        for src_path in &src_themes {
            let new_path = format!("ppt/theme/theme{}.xml", counters.theme);
            path_remap.insert(src_path.clone(), new_path);
            counters.theme += 1;
        }

        // Assign new names for slideMasters
        let src_masters: Vec<String> = sorted_keys(&src, "ppt/slideMasters/", ".xml");
        for src_path in &src_masters {
            let new_path = format!("ppt/slideMasters/slideMaster{}.xml", counters.master);
            path_remap.insert(src_path.clone(), new_path);
            counters.master += 1;
        }

        // Assign new names for slideLayouts
        let src_layouts: Vec<String> = sorted_keys(&src, "ppt/slideLayouts/", ".xml");
        for src_path in &src_layouts {
            let new_path = format!("ppt/slideLayouts/slideLayout{}.xml", counters.layout);
            path_remap.insert(src_path.clone(), new_path);
            counters.layout += 1;
        }

        // Scan ALL .rels in src to find media/resources and assign new names
        pre_scan_resources(&src, &mut path_remap, &mut counters)?;

        // === Phase 2: Copy all files using the complete remap ===

        // Copy themes
        for src_path in &src_themes {
            let new_path = path_remap.get(src_path).unwrap().clone();
            copy_file_with_rels(&src, &mut archive, &path_remap, src_path, &new_path, "ppt/theme")?;
            add_content_type(&mut archive, &format!("/{}", new_path), THEME_CT)?;
        }

        // Copy slideMasters
        for src_path in &src_masters {
            let new_path = path_remap.get(src_path).unwrap().clone();
            copy_file_with_rels(&src, &mut archive, &path_remap, src_path, &new_path, "ppt/slideMasters")?;
            add_content_type(&mut archive, &format!("/{}", new_path), MASTER_CT)?;

            // Register master in presentation.xml
            let pres = archive.get("ppt/presentation.xml").unwrap().clone();
            let rid_str = format!("rId{}", next_rid);
            let updated_pres = pptx::add_master_to_presentation_xml(&pres, next_master_id, &rid_str)?;
            archive.insert("ppt/presentation.xml".to_string(), updated_pres);

            // Add relationship in presentation.xml.rels
            let rels = archive.get("ppt/_rels/presentation.xml.rels").unwrap().clone();
            let master_target = new_path.strip_prefix("ppt/").unwrap_or(&new_path);
            let updated_rels = pptx::add_relationship_to_rels(&rels, &rid_str, MASTER_REL_TYPE, master_target)?;
            archive.insert("ppt/_rels/presentation.xml.rels".to_string(), updated_rels);

            next_rid += 1;
            next_master_id += 1;
        }

        // Copy slideLayouts
        for src_path in &src_layouts {
            let new_path = path_remap.get(src_path).unwrap().clone();
            copy_file_with_rels(&src, &mut archive, &path_remap, src_path, &new_path, "ppt/slideLayouts")?;
            add_content_type(&mut archive, &format!("/{}", new_path), LAYOUT_CT)?;
        }

        // === Phase 3: Copy slides ===
        for slide_info in &src_slides {
            let src_slide_rel_target = match rid_to_target.get(&slide_info.r_id) {
                Some(t) => t,
                None => continue,
            };

            let src_slide_path = normalize_ppt_path(src_slide_rel_target, "ppt");

            if !src.contains_key(&src_slide_path) {
                continue;
            }

            let new_slide_path = format!("ppt/slides/slide{}.xml", counters.slide);
            // Add to remap (in case other things reference this slide)
            path_remap.insert(src_slide_path.clone(), new_slide_path.clone());

            copy_file_with_rels(&src, &mut archive, &path_remap, &src_slide_path, &new_slide_path, "ppt/slides")?;

            // Add slide to presentation.xml
            let pres = archive.get("ppt/presentation.xml").unwrap().clone();
            let rid_str = format!("rId{}", next_rid);
            let updated_pres = pptx::add_slide_to_presentation_xml(&pres, next_slide_id, &rid_str)?;
            archive.insert("ppt/presentation.xml".to_string(), updated_pres);

            // Add relationship in presentation.xml.rels
            let rels = archive.get("ppt/_rels/presentation.xml.rels").unwrap().clone();
            let slide_target = format!("slides/slide{}.xml", counters.slide);
            let updated_rels = pptx::add_relationship_to_rels(&rels, &rid_str, SLIDE_REL_TYPE, &slide_target)?;
            archive.insert("ppt/_rels/presentation.xml.rels".to_string(), updated_rels);

            add_content_type(&mut archive, &format!("/ppt/slides/slide{}.xml", counters.slide), SLIDE_CT)?;

            next_slide_id += 1;
            next_rid += 1;
            counters.slide += 1;
        }
    }

    pptx::write_pptx(&archive, output)?;
    Ok(())
}

struct Counters {
    slide: u32,
    layout: u32,
    master: u32,
    theme: u32,
    media: u32,
    notes: u32,
    chart: u32,
}

/// Get sorted keys matching a directory prefix and suffix (excluding _rels).
fn sorted_keys(archive: &pptx::PptxArchive, dir: &str, suffix: &str) -> Vec<String> {
    let mut keys: Vec<String> = archive
        .keys()
        .filter(|k| k.starts_with(dir) && !k.contains("/_rels/") && k.ends_with(suffix))
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// Pre-scan all .rels files in the source to find resources (media, charts, notesSlides, etc.)
/// and assign them new names in path_remap.
fn pre_scan_resources(
    src: &pptx::PptxArchive,
    path_remap: &mut HashMap<String, String>,
    counters: &mut Counters,
) -> Result<()> {
    // Find all .rels files
    let rels_files: Vec<String> = src
        .keys()
        .filter(|k| k.ends_with(".rels"))
        .cloned()
        .collect();

    for rels_path in &rels_files {
        let rels_data = src.get(rels_path).unwrap();
        let rels = pptx::parse_rels(rels_data)?;

        // Determine the context directory for this .rels file
        // e.g. "ppt/slides/_rels/slide1.xml.rels" -> context is "ppt/slides"
        let context_dir = rels_context_dir(rels_path);

        for rel in &rels {
            if rel.target.starts_with("http://") || rel.target.starts_with("https://")
                || rel.target_mode.as_deref() == Some("External")
            {
                continue;
            }

            let abs_path = resolve_path(&context_dir, &rel.target);

            // Skip if already remapped or doesn't exist in source
            if path_remap.contains_key(&abs_path) || !src.contains_key(&abs_path) {
                continue;
            }

            // Assign new name based on type
            let new_path = if abs_path.starts_with("ppt/media/") {
                let ext = abs_path.rsplit('.').next().unwrap_or("bin");
                let p = format!("ppt/media/media{}.{}", counters.media, ext);
                counters.media += 1;
                p
            } else if abs_path.starts_with("ppt/notesSlides/") && abs_path.ends_with(".xml") {
                let p = format!("ppt/notesSlides/notesSlide{}.xml", counters.notes);
                counters.notes += 1;
                p
            } else if abs_path.starts_with("ppt/charts/") && abs_path.ends_with(".xml") {
                let p = format!("ppt/charts/chart{}.xml", counters.chart);
                counters.chart += 1;
                p
            } else if abs_path.starts_with("ppt/embeddings/") {
                let ext = abs_path.rsplit('.').next().unwrap_or("bin");
                let p = format!("ppt/embeddings/embed{}.{}", counters.media, ext);
                counters.media += 1;
                p
            } else if abs_path.starts_with("ppt/diagrams/") {
                let filename = abs_path.rsplit('/').next().unwrap_or("data.xml");
                let p = format!("ppt/diagrams/d{}_{}", counters.media, filename);
                counters.media += 1;
                p
            } else {
                // Generic fallback: keep in same directory structure with unique suffix
                continue; // Skip unknown types to avoid breaking things
            };

            path_remap.insert(abs_path, new_path);
        }
    }

    Ok(())
}

/// Copy a file and its .rels from src to dest, rewriting .rels targets using path_remap.
fn copy_file_with_rels(
    src: &pptx::PptxArchive,
    dest: &mut pptx::PptxArchive,
    path_remap: &HashMap<String, String>,
    src_path: &str,
    new_path: &str,
    context_dir: &str,
) -> Result<()> {
    // Copy the main file
    if let Some(data) = src.get(src_path) {
        dest.insert(new_path.to_string(), data.clone());
    }

    // Copy and rewrite .rels file
    let src_filename = src_path.rsplit('/').next().unwrap();
    let src_dir = src_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let src_rels_path = format!("{}/_rels/{}.rels", src_dir, src_filename);

    let new_filename = new_path.rsplit('/').next().unwrap();
    let new_dir = new_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let new_rels_path = format!("{}/_rels/{}.rels", new_dir, new_filename);

    if let Some(rels_data) = src.get(&src_rels_path) {
        let rels = pptx::parse_rels(rels_data)?;
        let mut entries: Vec<RelEntry> = Vec::new();

        for rel in &rels {
            let new_target = remap_target(context_dir, &rel.target, path_remap, new_dir);
            entries.push(RelEntry {
                id: rel.id.clone(),
                rel_type: rel.rel_type.clone(),
                target: new_target,
                target_mode: rel.target_mode.clone(),
            });
        }

        let new_rels_xml = build_rels_xml(&entries)?;
        dest.insert(new_rels_path, new_rels_xml);

        // Also copy referenced resources that have been remapped
        for rel in &rels {
            if rel.target.starts_with("http://") || rel.target.starts_with("https://") {
                continue;
            }
            let abs_path = resolve_path(context_dir, &rel.target);
            if let Some(new_abs) = path_remap.get(&abs_path) {
                if !dest.contains_key(new_abs) {
                    if let Some(data) = src.get(&abs_path) {
                        dest.insert(new_abs.clone(), data.clone());

                        // Add content type for known types
                        add_content_type_for_path(dest, new_abs)?;

                        // Recursively copy .rels for this resource (e.g. notesSlide has its own .rels)
                        let res_src_dir = abs_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                        let res_new_dir = new_abs.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                        let res_filename = abs_path.rsplit('/').next().unwrap();
                        let res_new_filename = new_abs.rsplit('/').next().unwrap();
                        let res_rels_path = format!("{}/_rels/{}.rels", res_src_dir, res_filename);

                        if let Some(res_rels_data) = src.get(&res_rels_path) {
                            let res_rels = pptx::parse_rels(res_rels_data)?;
                            let mut res_entries: Vec<RelEntry> = Vec::new();
                            for r in &res_rels {
                                let t = remap_target(res_src_dir, &r.target, path_remap, res_new_dir);
                                res_entries.push(RelEntry {
                                    id: r.id.clone(),
                                    rel_type: r.rel_type.clone(),
                                    target: t,
                                    target_mode: r.target_mode.clone(),
                                });

                                // Copy sub-resources too
                                if !r.target.starts_with("http") {
                                    let sub_abs = resolve_path(res_src_dir, &r.target);
                                    if let Some(new_sub) = path_remap.get(&sub_abs) {
                                        if !dest.contains_key(new_sub) {
                                            if let Some(d) = src.get(&sub_abs) {
                                                dest.insert(new_sub.clone(), d.clone());
                                                add_content_type_for_path(dest, new_sub)?;
                                            }
                                        }
                                    }
                                }
                            }
                            let new_res_rels = build_rels_xml(&res_entries)?;
                            let new_res_rels_path = format!("{}/_rels/{}.rels", res_new_dir, res_new_filename);
                            dest.insert(new_res_rels_path, new_res_rels);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Remap a relationship target using path_remap.
fn remap_target(
    src_context: &str,
    original_target: &str,
    path_remap: &HashMap<String, String>,
    new_context: &str,
) -> String {
    if original_target.starts_with("http://") || original_target.starts_with("https://") {
        return original_target.to_string();
    }

    let abs_path = resolve_path(src_context, original_target);

    if let Some(new_abs) = path_remap.get(&abs_path) {
        make_relative(new_context, new_abs)
    } else {
        // Not remapped - keep original (it might be referencing something in the base file)
        original_target.to_string()
    }
}

/// Add content type override for a file path based on its extension/location.
fn add_content_type_for_path(archive: &mut pptx::PptxArchive, path: &str) -> Result<()> {
    let ct = if path.starts_with("ppt/notesSlides/") && path.ends_with(".xml") {
        Some(NOTES_SLIDE_CT)
    } else if path.starts_with("ppt/charts/") && path.ends_with(".xml") {
        Some(CHART_CT)
    } else {
        None // Media files are handled by Default extensions in Content_Types
    };

    if let Some(content_type) = ct {
        add_content_type(archive, &format!("/{}", path), content_type)?;
    }
    Ok(())
}

struct RelEntry {
    id: String,
    rel_type: String,
    target: String,
    target_mode: Option<String>,
}

/// Build a .rels XML from a list of RelEntry.
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

fn add_content_type(archive: &mut pptx::PptxArchive, part_name: &str, content_type: &str) -> Result<()> {
    let ct = archive.get("[Content_Types].xml")
        .context("[Content_Types].xml が見つかりません")?
        .clone();
    let updated = pptx::add_content_type_override(&ct, part_name, content_type)?;
    archive.insert("[Content_Types].xml".to_string(), updated);
    Ok(())
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

fn make_relative(context_dir: &str, target_abs: &str) -> String {
    let ctx_parts: Vec<&str> = context_dir.split('/').collect();
    let tgt_parts: Vec<&str> = target_abs.split('/').collect();

    let common = ctx_parts.iter().zip(tgt_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let ups = ctx_parts.len() - common;
    let mut result = String::new();
    for _ in 0..ups {
        result.push_str("../");
    }
    result.push_str(&tgt_parts[common..].join("/"));
    result
}

/// Extract the context directory from a .rels file path.
/// e.g. "ppt/slides/_rels/slide1.xml.rels" -> "ppt/slides"
fn rels_context_dir(rels_path: &str) -> String {
    // Remove _rels/filename.rels to get the parent dir
    if let Some(pos) = rels_path.rfind("/_rels/") {
        rels_path[..pos].to_string()
    } else {
        String::new()
    }
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

/// Parse the highest sldMasterId from presentation.xml.
fn parse_max_master_id(xml: &[u8]) -> u32 {
    // Master IDs typically start at 2147483648
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut max_id: u32 = 2147483648;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = e.name().as_ref().to_vec();
                if local_name(&name) == b"sldMasterId" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            if let Ok(val) = std::str::from_utf8(&attr.value) {
                                if let Ok(id) = val.parse::<u32>() {
                                    if id >= max_id {
                                        max_id = id;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    max_id
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}
