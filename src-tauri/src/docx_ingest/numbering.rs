use super::{
    model::{DocxBlock, DocxDocumentModel, DocxParagraph, DocxTable},
    xml::parse_xml,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxNumberingLevel {
    pub(crate) level: u32,
    pub(crate) format: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) start: Option<i32>,
    pub(crate) left_indent_twips: Option<i32>,
    pub(crate) hanging_indent_twips: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxNumberingOverride {
    pub(crate) level: u32,
    pub(crate) start_override: Option<i32>,
    pub(crate) level_override: Option<DocxNumberingLevel>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxNumberingResolved {
    pub(crate) num_id: String,
    pub(crate) abstract_id: Option<String>,
    pub(crate) level: u32,
    pub(crate) format: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) start: Option<i32>,
    pub(crate) left_indent_twips: Option<i32>,
    pub(crate) hanging_indent_twips: Option<i32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxNumberingCatalog {
    pub(crate) abstract_levels: BTreeMap<(String, u32), DocxNumberingLevel>,
    pub(crate) instances: BTreeMap<String, (String, BTreeMap<u32, DocxNumberingOverride>)>,
}

impl DocxNumberingCatalog {
    pub(crate) fn parse(numbering_xml: &[u8]) -> Result<Self, String> {
        let root = parse_xml(numbering_xml, "word/numbering.xml")?;
        let mut catalog = Self::default();
        for abstract_num in root.children_named("abstractNum") {
            let abstract_id = abstract_num.attr("abstractNumId").unwrap_or_default();
            if abstract_id.is_empty() {
                continue;
            }
            for level in abstract_num.children_named("lvl") {
                let ilvl = level
                    .attr("ilvl")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                catalog
                    .abstract_levels
                    .insert((abstract_id.to_string(), ilvl), parse_level(ilvl, level));
            }
        }
        for num in root.children_named("num") {
            let num_id = num.attr("numId").unwrap_or_default();
            let Some(abstract_id) = num.child("abstractNumId").and_then(|node| node.attr("val"))
            else {
                continue;
            };
            let mut overrides = BTreeMap::new();
            for override_node in num.children_named("lvlOverride") {
                let level = override_node
                    .attr("ilvl")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let start_override = override_node
                    .child("startOverride")
                    .and_then(|node| node.attr("val"))
                    .and_then(|value| value.parse().ok());
                let level_override = override_node
                    .child("lvl")
                    .map(|node| parse_level(level, node));
                overrides.insert(
                    level,
                    DocxNumberingOverride {
                        level,
                        start_override,
                        level_override,
                    },
                );
            }
            if !num_id.is_empty() {
                catalog
                    .instances
                    .insert(num_id.to_string(), (abstract_id.to_string(), overrides));
            }
        }
        Ok(catalog)
    }

    pub(crate) fn resolve(
        &self,
        num_id: Option<&str>,
        level: Option<u32>,
    ) -> Option<DocxNumberingResolved> {
        let num_id = num_id?;
        let level = level.unwrap_or(0);
        let (abstract_id, overrides) = self.instances.get(num_id)?;
        let base = self
            .abstract_levels
            .get(&(abstract_id.clone(), level))
            .cloned();
        let override_node = overrides.get(&level);
        let level_def = override_node
            .and_then(|item| item.level_override.clone())
            .or(base.clone())?;
        Some(DocxNumberingResolved {
            num_id: num_id.to_string(),
            abstract_id: Some(abstract_id.clone()),
            level,
            format: level_def.format,
            text: level_def.text,
            start: override_node
                .and_then(|item| item.start_override)
                .or(level_def.start),
            left_indent_twips: level_def.left_indent_twips,
            hanging_indent_twips: level_def.hanging_indent_twips,
        })
    }
}

pub(crate) fn assign_numbering_labels(
    model: &mut DocxDocumentModel,
    catalog: &DocxNumberingCatalog,
) {
    fn assign_table(
        table: &mut DocxTable,
        catalog: &DocxNumberingCatalog,
        counters: &mut BTreeMap<String, BTreeMap<u32, i32>>,
    ) {
        for row in &mut table.rows {
            for cell in &mut row.cells {
                for paragraph in &mut cell.paragraphs {
                    assign_paragraph_label(paragraph, catalog, counters);
                }
                for nested in &mut cell.tables {
                    assign_table(nested, catalog, counters);
                }
            }
        }
    }

    let mut counters = BTreeMap::<String, BTreeMap<u32, i32>>::new();
    for block in &mut model.blocks {
        match block {
            DocxBlock::Paragraph(paragraph) => {
                assign_paragraph_label(paragraph, catalog, &mut counters)
            }
            DocxBlock::Table(table) => assign_table(table, catalog, &mut counters),
        }
    }
    for text_box in &mut model.text_boxes {
        for paragraph in &mut text_box.paragraphs {
            assign_paragraph_label(paragraph, catalog, &mut counters);
        }
    }
}

fn assign_paragraph_label(
    paragraph: &mut DocxParagraph,
    catalog: &DocxNumberingCatalog,
    counters: &mut BTreeMap<String, BTreeMap<u32, i32>>,
) {
    let Some(numbering) = paragraph.resolved_numbering.as_ref() else {
        return;
    };
    let levels = counters.entry(numbering.num_id.clone()).or_default();
    levels.retain(|level, _| *level <= numbering.level);
    let value = levels
        .entry(numbering.level)
        .and_modify(|value| *value = value.saturating_add(1))
        .or_insert_with(|| numbering.start.unwrap_or(1));
    let current_value = *value;

    let template = numbering.text.as_deref().unwrap_or("%1.");
    if numbering.format.as_deref() == Some("bullet") {
        paragraph.numbering_label = Some(template.to_string());
        return;
    }
    let mut label = template.to_string();
    for level in 0..=8_u32 {
        let placeholder = format!("%{}", level + 1);
        if !label.contains(&placeholder) {
            continue;
        }
        let level_value = if level == numbering.level {
            current_value
        } else {
            *levels.entry(level).or_insert_with(|| {
                catalog
                    .resolve(Some(&numbering.num_id), Some(level))
                    .and_then(|resolved| resolved.start)
                    .unwrap_or(1)
            })
        };
        let format = catalog
            .resolve(Some(&numbering.num_id), Some(level))
            .and_then(|resolved| resolved.format)
            .or_else(|| {
                (level == numbering.level)
                    .then(|| numbering.format.clone())
                    .flatten()
            });
        label = label.replace(
            &placeholder,
            &format_counter(level_value, format.as_deref()),
        );
    }
    paragraph.numbering_label = Some(label);
}

fn format_counter(value: i32, format: Option<&str>) -> String {
    match format.unwrap_or("decimal") {
        "upperLetter" => alphabetic(value, true),
        "lowerLetter" => alphabetic(value, false),
        "upperRoman" => roman(value).to_ascii_uppercase(),
        "lowerRoman" => roman(value).to_ascii_lowercase(),
        "decimalZero" => format!("{value:02}"),
        _ => value.to_string(),
    }
}

fn alphabetic(value: i32, uppercase: bool) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let mut value = value as u32;
    let mut output = Vec::new();
    while value > 0 {
        value -= 1;
        output.push((b'a' + (value % 26) as u8) as char);
        value /= 26;
    }
    let output = output.into_iter().rev().collect::<String>();
    if uppercase {
        output.to_ascii_uppercase()
    } else {
        output
    }
}

fn roman(value: i32) -> String {
    if !(1..=3999).contains(&value) {
        return value.to_string();
    }
    let mut remaining = value;
    let mut output = String::new();
    for (amount, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while remaining >= amount {
            remaining -= amount;
            output.push_str(symbol);
        }
    }
    output
}

fn parse_level(level: u32, node: &super::xml::XmlNode) -> DocxNumberingLevel {
    DocxNumberingLevel {
        level,
        format: node
            .child("numFmt")
            .and_then(|item| item.attr("val"))
            .map(str::to_string),
        text: node
            .child("lvlText")
            .and_then(|item| item.attr("val"))
            .map(str::to_string),
        start: node
            .child("start")
            .and_then(|item| item.attr("val"))
            .and_then(|value| value.parse().ok()),
        left_indent_twips: node
            .child("pPr")
            .and_then(|ppr| ppr.child("ind"))
            .and_then(|ind| ind.attr("left"))
            .and_then(|value| value.parse().ok()),
        hanging_indent_twips: node
            .child("pPr")
            .and_then(|ppr| ppr.child("ind"))
            .and_then(|ind| ind.attr("hanging"))
            .and_then(|value| value.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::{alphabetic, format_counter, roman, DocxNumberingCatalog};

    #[test]
    fn resolves_numbering_instance_and_level_override() {
        let catalog = DocxNumberingCatalog::parse(
            br#"<w:numbering xmlns:w="urn:w">
                <w:abstractNum w:abstractNumId="4"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="upperLetter"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>
                <w:num w:numId="9"><w:abstractNumId w:val="4"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="3"/></w:lvlOverride></w:num>
            </w:numbering>"#,
        )
        .unwrap();
        let resolved = catalog.resolve(Some("9"), Some(0)).unwrap();
        assert_eq!(resolved.abstract_id.as_deref(), Some("4"));
        assert_eq!(resolved.format.as_deref(), Some("upperLetter"));
        assert_eq!(resolved.start, Some(3));
        assert_eq!(resolved.hanging_indent_twips, Some(360));
    }

    #[test]
    fn formats_word_counter_styles_at_boundaries() {
        assert_eq!(alphabetic(1, true), "A");
        assert_eq!(alphabetic(26, false), "z");
        assert_eq!(alphabetic(27, true), "AA");
        assert_eq!(roman(49), "XLIX");
        assert_eq!(format_counter(9, Some("decimalZero")), "09");
        assert_eq!(format_counter(14, Some("lowerRoman")), "xiv");
    }
}
