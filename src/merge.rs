use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::pptx;

const SLIDE_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const SLIDE_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";

/// Merge multiple PPTX files into one output file.
/// The first file serves as the base; slides from subsequent files are appended.
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

    // Determine the highest existing slide number in the base
    let mut next_slide_num = find_max_slide_number(&archive) + 1;

    // Determine global media counter
    let mut next_media_num = find_max_media_number(&archive) + 1;

    // Find the base file's first slideLayout target (for rewriting imported slide layouts)
    let base_layout_target = find_first_slide_layout_target(&archive);

    // Process each additional file
    for file_path in &input_files[1..] {
        let file_path = file_path.as_ref();
        let src_archive = pptx::read_pptx(file_path)
            .with_context(|| format!("ファイルの読み込み失敗: {}", file_path.display()))?;

        // Parse source presentation
        let src_pres_xml = src_archive
            .get("ppt/presentation.xml")
            .with_context(|| {
                format!(
                    "{}にppt/presentation.xmlがありません",
                    file_path.display()
                )
            })?;
        let src_slides = pptx::parse_slide_list(src_pres_xml)?;

        // Parse source presentation rels to map rId -> target
        let src_pres_rels_xml = src_archive
            .get("ppt/_rels/presentation.xml.rels")
            .with_context(|| {
                format!(
                    "{}にpresentation.xml.relsがありません",
                    file_path.display()
                )
            })?;
        let src_rels = pptx::parse_rels(src_pres_rels_xml)?;
        let rid_to_target: HashMap<String, String> = src_rels
            .iter()
            .map(|r| (r.id.clone(), r.target.clone()))
            .collect();

        // Process each slide from this source file
        for slide_info in &src_slides {
            let src_slide_target = match rid_to_target.get(&slide_info.r_id) {
                Some(t) => t,
                None => continue,
            };

            // Source slide path (e.g. "slides/slide1.xml" -> "ppt/slides/slide1.xml")
            let src_slide_path = if src_slide_target.starts_with("ppt/") {
                src_slide_target.clone()
            } else {
                format!("ppt/{}", src_slide_target)
            };

            // Read source slide XML
            let slide_xml = match src_archive.get(&src_slide_path) {
                Some(data) => data.clone(),
                None => continue,
            };

            // New slide path in output
            let new_slide_name = format!("ppt/slides/slide{}.xml", next_slide_num);
            let new_slide_rels_name =
                format!("ppt/slides/_rels/slide{}.xml.rels", next_slide_num);

            // Handle slide relationships (images, layout, etc.)
            let src_slide_rels_path = format!(
                "ppt/slides/_rels/{}.rels",
                src_slide_target
                    .rsplit('/')
                    .next()
                    .unwrap_or("slide1.xml")
            );
            let slide_rels_xml = src_archive.get(&src_slide_rels_path);

            let mut media_renames: HashMap<String, String> = HashMap::new();

            if let Some(rels_data) = slide_rels_xml {
                // Parse slide rels to find media references
                let slide_rels = pptx::parse_rels(rels_data)?;
                for rel in &slide_rels {
                    if rel.rel_type.ends_with("/image")
                        || rel.rel_type.ends_with("/audio")
                        || rel.rel_type.ends_with("/video")
                    {
                        // Copy the media file with a new name
                        let src_media_rel = &rel.target; // e.g. "../media/image1.png"
                        let extension = src_media_rel
                            .rsplit('.')
                            .next()
                            .unwrap_or("bin");

                        // Resolve relative path from ppt/slides/ context
                        let src_media_path = resolve_media_path("ppt/slides", src_media_rel);

                        if let Some(media_data) = src_archive.get(&src_media_path) {
                            let new_media_full =
                                format!("ppt/media/image{}.{}", next_media_num, extension);

                            archive.insert(new_media_full, media_data.clone());
                            media_renames.insert(
                                src_media_rel.to_string(),
                                format!("../media/image{}.{}", next_media_num, extension),
                            );
                            next_media_num += 1;
                        }
                    }
                }

                // Rewrite the slide rels file
                let new_rels = pptx::rewrite_slide_rels(
                    rels_data,
                    &media_renames,
                    &base_layout_target,
                )?;
                archive.insert(new_slide_rels_name, new_rels);
            }

            // Add slide XML to archive
            archive.insert(new_slide_name.clone(), slide_xml);

            // Update presentation.xml - add slide reference
            let current_pres = archive.get("ppt/presentation.xml").unwrap().clone();
            let new_rid = format!("rId{}", next_rid);
            let updated_pres =
                pptx::add_slide_to_presentation_xml(&current_pres, next_slide_id, &new_rid)?;
            archive.insert("ppt/presentation.xml".to_string(), updated_pres);

            // Update presentation.xml.rels - add relationship
            let current_rels = archive
                .get("ppt/_rels/presentation.xml.rels")
                .unwrap()
                .clone();
            let slide_target = format!("slides/slide{}.xml", next_slide_num);
            let updated_rels = pptx::add_relationship_to_rels(
                &current_rels,
                &new_rid,
                SLIDE_REL_TYPE,
                &slide_target,
            )?;
            archive.insert("ppt/_rels/presentation.xml.rels".to_string(), updated_rels);

            // Update [Content_Types].xml
            let current_ct = archive.get("[Content_Types].xml").unwrap().clone();
            let part_name = format!("/ppt/slides/slide{}.xml", next_slide_num);
            let updated_ct =
                pptx::add_content_type_override(&current_ct, &part_name, SLIDE_CONTENT_TYPE)?;
            archive.insert("[Content_Types].xml".to_string(), updated_ct);

            next_slide_id += 1;
            next_rid += 1;
            next_slide_num += 1;
        }
    }

    // Write the merged archive
    pptx::write_pptx(&archive, output)?;
    Ok(())
}

/// Find the highest slide number in existing slide file names (e.g. slide3.xml -> 3).
fn find_max_slide_number(archive: &pptx::PptxArchive) -> u32 {
    archive
        .keys()
        .filter_map(|name| {
            let fname = name.rsplit('/').next()?;
            let num_str = fname.strip_prefix("slide")?.strip_suffix(".xml")?;
            num_str.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// Find the highest media number in existing media file names.
fn find_max_media_number(archive: &pptx::PptxArchive) -> u32 {
    archive
        .keys()
        .filter_map(|name| {
            if !name.contains("ppt/media/") {
                return None;
            }
            let fname = name.rsplit('/').next()?;
            // Extract number from names like "image1.png", "image12.jpeg"
            let without_ext = fname.rsplit('.').last()?;
            let num_str = without_ext
                .trim_start_matches(|c: char| c.is_alphabetic());
            num_str.parse::<u32>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// Find the first slideLayout target from the base presentation's first slide.
fn find_first_slide_layout_target(archive: &pptx::PptxArchive) -> String {
    // Try to find slide1's rels and get its layout
    if let Some(rels_data) = archive.get("ppt/slides/_rels/slide1.xml.rels") {
        if let Ok(rels) = pptx::parse_rels(rels_data) {
            for rel in &rels {
                if rel.rel_type.ends_with("/slideLayout") {
                    return rel.target.clone();
                }
            }
        }
    }
    // Fallback
    "../slideLayouts/slideLayout1.xml".to_string()
}

/// Resolve a relative path from a base directory context.
/// e.g. resolve_media_path("ppt/slides", "../media/image1.png") -> "ppt/media/image1.png"
fn resolve_media_path(base: &str, relative: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').collect();
    for segment in relative.split('/') {
        match segment {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}
