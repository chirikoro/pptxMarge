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
const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

/// Merge multiple PPTX files into one output file.
pub fn merge_pptx_files(input_files: &[impl AsRef<Path>], output: &Path) -> Result<()> {
    if input_files.len() < 2 {
        anyhow::bail!("結合するには2つ以上のファイルが必要です");
    }

    // Read the base file
    let base_path = input_files[0].as_ref();
    let mut archive = pptx::read_pptx(base_path)
        .with_context(|| format!("ベースファイルの読み込み失敗: {}", base_path.display()))?;

    // Parse base presentation to find current max IDs
    let pres_xml = archive
        .get("ppt/presentation.xml")
        .context("ベースファイルにppt/presentation.xmlがありません")?
        .clone();
    let base_slides = pptx::parse_slide_list(&pres_xml)?;
    let mut next_slide_id = pptx::max_slide_id(&base_slides) + 1;

    let pres_rels_xml = archive
        .get("ppt/_rels/presentation.xml.rels")
        .context("ベースファイルにpresentation.xml.relsがありません")?
        .clone();
    let base_rels = pptx::parse_rels(&pres_rels_xml)?;
    let mut next_rid = pptx::max_rid(&base_rels) + 1;

    // Global counters for unique naming
    let mut next_slide_num = find_max_number(&archive, "ppt/slides/", "slide", ".xml") + 1;
    let mut next_layout_num = find_max_number(&archive, "ppt/slideLayouts/", "slideLayout", ".xml") + 1;
    let mut next_master_num = find_max_number(&archive, "ppt/slideMasters/", "slideMaster", ".xml") + 1;
    let mut next_theme_num = find_max_number(&archive, "ppt/theme/", "theme", ".xml") + 1;
    let mut next_media_num = find_max_media_number(&archive) + 1;

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

        // Track renames: old archive path -> new archive path
        // This is used to remap references throughout the dependency chain.
        let mut path_remap: HashMap<String, String> = HashMap::new();

        // Phase 1: Copy slideMasters, slideLayouts, themes, and media from source
        // so that imported slides can reference them.
        copy_masters_layouts_themes(
            &src,
            &mut archive,
            &mut path_remap,
            &mut next_layout_num,
            &mut next_master_num,
            &mut next_theme_num,
            &mut next_media_num,
        )?;

        // Phase 2: Copy each slide
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
            path_remap.insert(src_slide_path.clone(), new_slide_path.clone());

            // Copy the slide XML
            archive.insert(new_slide_path.clone(), slide_xml);

            // Copy and rewrite slide's .rels file
            let src_slide_filename = src_slide_path.rsplit('/').next().unwrap_or("slide1.xml");
            let src_slide_rels_path = format!("ppt/slides/_rels/{}.rels", src_slide_filename);
            let new_slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", next_slide_num);

            if let Some(rels_data) = src.get(&src_slide_rels_path) {
                let new_rels = rewrite_rels_with_remap(
                    rels_data,
                    &src,
                    &mut archive,
                    &mut path_remap,
                    &mut next_media_num,
                    "ppt/slides",
                )?;
                archive.insert(new_slide_rels_path, new_rels);
            }

            // Add slide to presentation.xml
            let current_pres = archive.get("ppt/presentation.xml").unwrap().clone();
            let new_rid_str = format!("rId{}", next_rid);
            let updated_pres =
                pptx::add_slide_to_presentation_xml(&current_pres, next_slide_id, &new_rid_str)?;
            archive.insert("ppt/presentation.xml".to_string(), updated_pres);

            // Add relationship to presentation.xml.rels
            let current_rels = archive
                .get("ppt/_rels/presentation.xml.rels")
                .unwrap()
                .clone();
            let slide_target = format!("slides/slide{}.xml", next_slide_num);
            let updated_rels = pptx::add_relationship_to_rels(
                &current_rels,
                &new_rid_str,
                SLIDE_REL_TYPE,
                &slide_target,
            )?;
            archive.insert("ppt/_rels/presentation.xml.rels".to_string(), updated_rels);

            // Add content type for the slide
            add_content_type(&mut archive, &format!("/ppt/slides/slide{}.xml", next_slide_num), SLIDE_CT)?;

            next_slide_id += 1;
            next_rid += 1;
            next_slide_num += 1;
        }
    }

    // Write the merged archive
    pptx::write_pptx(&archive, output)?;
    Ok(())
}

/// Copy slideMasters, slideLayouts, and themes from a source archive into the output archive.
/// Populates path_remap so slide rels can find the new paths.
fn copy_masters_layouts_themes(
    src: &pptx::PptxArchive,
    dest: &mut pptx::PptxArchive,
    path_remap: &mut HashMap<String, String>,
    next_layout: &mut u32,
    next_master: &mut u32,
    next_theme: &mut u32,
    next_media: &mut u32,
) -> Result<()> {
    // Collect source slideMasters
    let src_masters: Vec<String> = src
        .keys()
        .filter(|k| k.starts_with("ppt/slideMasters/") && !k.contains("/_rels/") && k.ends_with(".xml"))
        .cloned()
        .collect();

    for src_master_path in &src_masters {
        let new_master_path = format!("ppt/slideMasters/slideMaster{}.xml", *next_master);
        path_remap.insert(src_master_path.clone(), new_master_path.clone());

        // Copy master XML
        if let Some(data) = src.get(src_master_path) {
            dest.insert(new_master_path.clone(), data.clone());
        }

        // Copy and rewrite master's .rels
        let master_filename = src_master_path.rsplit('/').next().unwrap();
        let src_master_rels = format!("ppt/slideMasters/_rels/{}.rels", master_filename);
        let new_master_rels = format!("ppt/slideMasters/_rels/slideMaster{}.xml.rels", *next_master);

        if let Some(rels_data) = src.get(&src_master_rels) {
            let new_rels = rewrite_rels_with_remap(
                rels_data, src, dest, path_remap, next_media, "ppt/slideMasters",
            )?;
            dest.insert(new_master_rels, new_rels);
        }

        add_content_type(dest, &format!("/ppt/slideMasters/slideMaster{}.xml", *next_master), MASTER_CT)?;
        *next_master += 1;
    }

    // Collect source slideLayouts
    let src_layouts: Vec<String> = src
        .keys()
        .filter(|k| k.starts_with("ppt/slideLayouts/") && !k.contains("/_rels/") && k.ends_with(".xml"))
        .cloned()
        .collect();

    for src_layout_path in &src_layouts {
        let new_layout_path = format!("ppt/slideLayouts/slideLayout{}.xml", *next_layout);
        path_remap.insert(src_layout_path.clone(), new_layout_path.clone());

        if let Some(data) = src.get(src_layout_path) {
            dest.insert(new_layout_path.clone(), data.clone());
        }

        // Copy and rewrite layout's .rels
        let layout_filename = src_layout_path.rsplit('/').next().unwrap();
        let src_layout_rels = format!("ppt/slideLayouts/_rels/{}.rels", layout_filename);
        let new_layout_rels = format!("ppt/slideLayouts/_rels/slideLayout{}.xml.rels", *next_layout);

        if let Some(rels_data) = src.get(&src_layout_rels) {
            let new_rels = rewrite_rels_with_remap(
                rels_data, src, dest, path_remap, next_media, "ppt/slideLayouts",
            )?;
            dest.insert(new_layout_rels, new_rels);
        }

        add_content_type(dest, &format!("/ppt/slideLayouts/slideLayout{}.xml", *next_layout), LAYOUT_CT)?;
        *next_layout += 1;
    }

    // Collect source themes
    let src_themes: Vec<String> = src
        .keys()
        .filter(|k| k.starts_with("ppt/theme/") && !k.contains("/_rels/") && k.ends_with(".xml"))
        .cloned()
        .collect();

    for src_theme_path in &src_themes {
        let new_theme_path = format!("ppt/theme/theme{}.xml", *next_theme);
        path_remap.insert(src_theme_path.clone(), new_theme_path.clone());

        if let Some(data) = src.get(src_theme_path) {
            dest.insert(new_theme_path.clone(), data.clone());
        }

        // Copy theme .rels if it exists
        let theme_filename = src_theme_path.rsplit('/').next().unwrap();
        let src_theme_rels = format!("ppt/theme/_rels/{}.rels", theme_filename);
        let new_theme_rels = format!("ppt/theme/_rels/theme{}.xml.rels", *next_theme);

        if let Some(rels_data) = src.get(&src_theme_rels) {
            let new_rels = rewrite_rels_with_remap(
                rels_data, src, dest, path_remap, next_media, "ppt/theme",
            )?;
            dest.insert(new_theme_rels, new_rels);
        }

        add_content_type(dest, &format!("/ppt/theme/theme{}.xml", *next_theme), THEME_CT)?;
        *next_theme += 1;
    }

    Ok(())
}

/// Rewrite a .rels file: for each relationship, remap the Target path using path_remap.
/// For resources not yet in the remap (media, charts, etc.), copy them to dest with new names.
fn rewrite_rels_with_remap(
    rels_xml: &[u8],
    src: &pptx::PptxArchive,
    dest: &mut pptx::PptxArchive,
    path_remap: &mut HashMap<String, String>,
    next_media: &mut u32,
    context_dir: &str,  // e.g. "ppt/slides" - the directory of the file owning this .rels
) -> Result<Vec<u8>> {
    let rels = pptx::parse_rels(rels_xml)?;
    let mut new_rels_entries: Vec<(String, String, String)> = Vec::new(); // (id, type, new_target)

    for rel in &rels {
        let src_abs = resolve_path(context_dir, &rel.target);
        let new_target = if let Some(new_abs) = path_remap.get(&src_abs) {
            // Already remapped (layout, master, theme, etc.)
            make_relative(context_dir, new_abs)
        } else if rel.target.starts_with("http://") || rel.target.starts_with("https://") {
            // External URL - keep as-is
            rel.target.clone()
        } else if src.contains_key(&src_abs) {
            // Resource exists in source - copy it with a new name
            let new_abs = copy_resource(src, dest, &src_abs, next_media)?;
            let relative = make_relative(context_dir, &new_abs);
            path_remap.insert(src_abs.clone(), new_abs);
            relative
        } else {
            // Resource not found in source - keep original target
            rel.target.clone()
        };

        new_rels_entries.push((rel.id.clone(), rel.rel_type.clone(), new_target));
    }

    // Build new .rels XML
    build_rels_xml(&new_rels_entries)
}

/// Copy a resource file from src to dest with a new unique name.
/// Returns the new absolute path in the archive.
fn copy_resource(
    src: &pptx::PptxArchive,
    dest: &mut pptx::PptxArchive,
    src_path: &str,
    next_media: &mut u32,
) -> Result<String> {
    let data = src.get(src_path)
        .with_context(|| format!("リソースが見つかりません: {}", src_path))?
        .clone();

    let extension = src_path.rsplit('.').next().unwrap_or("bin");
    let new_path = format!("ppt/media/resource{}.{}", *next_media, extension);
    *next_media += 1;

    dest.insert(new_path.clone(), data);
    Ok(new_path)
}

/// Build a .rels XML from a list of (Id, Type, Target) tuples.
fn build_rels_xml(entries: &[(String, String, String)]) -> Result<Vec<u8>> {
    use quick_xml::events::{BytesDecl, BytesStart, Event};
    use quick_xml::writer::Writer;

    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), Some("yes"))))?;

    let mut root = BytesStart::new("Relationships");
    root.push_attribute(("xmlns", "http://schemas.openxmlformats.org/package/2006/relationships"));
    writer.write_event(Event::Start(root))?;

    for (id, rel_type, target) in entries {
        let mut elem = BytesStart::new("Relationship");
        elem.push_attribute(("Id", id.as_str()));
        elem.push_attribute(("Type", rel_type.as_str()));
        elem.push_attribute(("Target", target.as_str()));
        writer.write_event(Event::Empty(elem))?;
    }

    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("Relationships")))?;
    Ok(writer.into_inner())
}

/// Helper: add a Content-Type override to [Content_Types].xml in the archive.
fn add_content_type(archive: &mut pptx::PptxArchive, part_name: &str, content_type: &str) -> Result<()> {
    let ct = archive.get("[Content_Types].xml")
        .context("[Content_Types].xml が見つかりません")?
        .clone();
    let updated = pptx::add_content_type_override(&ct, part_name, content_type)?;
    archive.insert("[Content_Types].xml".to_string(), updated);
    Ok(())
}

/// Normalize a relative path used in presentation.xml.rels to an absolute archive path.
/// e.g. "slides/slide1.xml" with base "ppt" -> "ppt/slides/slide1.xml"
fn normalize_ppt_path(rel_target: &str, base: &str) -> String {
    if rel_target.starts_with("ppt/") || rel_target.starts_with("/ppt/") {
        rel_target.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", base, rel_target)
    }
}

/// Resolve a relative path to an absolute archive path.
/// e.g. resolve_path("ppt/slides", "../media/image1.png") -> "ppt/media/image1.png"
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

/// Make a relative path from context_dir to target_abs.
/// e.g. make_relative("ppt/slides", "ppt/slideLayouts/slideLayout5.xml")
///      -> "../slideLayouts/slideLayout5.xml"
fn make_relative(context_dir: &str, target_abs: &str) -> String {
    let ctx_parts: Vec<&str> = context_dir.split('/').collect();
    let tgt_parts: Vec<&str> = target_abs.split('/').collect();

    // Find common prefix length
    let common = ctx_parts.iter().zip(tgt_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let ups = ctx_parts.len() - common;
    let mut result = String::new();
    for _ in 0..ups {
        result.push_str("../");
    }
    let remaining: Vec<&str> = tgt_parts[common..].to_vec();
    result.push_str(&remaining.join("/"));
    result
}

/// Find the highest N in files matching {dir}{prefix}N{suffix}
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

/// Find the highest media number (handles various prefixes like image, media, etc.).
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
