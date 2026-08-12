use quick_xml::{escape::unescape, events::Event, Reader};
use std::collections::BTreeMap;

const DEFAULT_MAX_XML_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_XML_DEPTH: usize = 256;
const DEFAULT_MAX_XML_NODES: usize = 250_000;
const DEFAULT_MAX_XML_ATTRIBUTES: usize = 64;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct XmlNode {
    pub(crate) name: String,
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) children: Vec<XmlNode>,
    pub(crate) text: String,
}

impl XmlNode {
    pub(crate) fn is(&self, local_name: &str) -> bool {
        local_name_of(&self.name).eq_ignore_ascii_case(local_name)
    }

    pub(crate) fn attr(&self, local_name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| local_name_of(name).eq_ignore_ascii_case(local_name))
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn child(&self, local_name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|child| child.is(local_name))
    }

    pub(crate) fn children_named<'a>(&'a self, local_name: &str) -> Vec<&'a XmlNode> {
        self.children
            .iter()
            .filter(|child| child.is(local_name))
            .collect()
    }

    pub(crate) fn descendants_named<'a>(&'a self, local_name: &str) -> Vec<&'a XmlNode> {
        let mut result = Vec::new();
        self.collect_descendants(local_name, &mut result);
        result
    }

    fn collect_descendants<'a>(&'a self, local_name: &str, result: &mut Vec<&'a XmlNode>) {
        for child in &self.children {
            if child.is(local_name) {
                result.push(child);
            }
            child.collect_descendants(local_name, result);
        }
    }

    pub(crate) fn text_content(&self) -> String {
        let mut value = self.text.clone();
        for child in &self.children {
            value.push_str(&child.text_content());
        }
        value
    }
}

pub(crate) fn local_name(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

pub(crate) fn local_name_of(name: &str) -> &str {
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
}

pub(crate) fn parse_xml(xml: &[u8], label: &str) -> Result<XmlNode, String> {
    if xml.len() > DEFAULT_MAX_XML_BYTES {
        return Err(format!(
            "docx_xml_size_limit:{}:{}>{}",
            label,
            xml.len(),
            DEFAULT_MAX_XML_BYTES
        ));
    }
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut stack = vec![XmlNode {
        name: "__root__".to_string(),
        ..XmlNode::default()
    }];
    let mut node_count = 0_usize;

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format!("docx_xml_tree_parse_failed:{}:{}", label, error))?
        {
            Event::Start(event) => {
                node_count = node_count.saturating_add(1);
                if node_count > DEFAULT_MAX_XML_NODES {
                    return Err(format!("docx_xml_node_limit:{}", label));
                }
                if stack.len() >= DEFAULT_MAX_XML_DEPTH {
                    return Err(format!("docx_xml_depth_limit:{}", label));
                }
                stack.push(node_from_start(&event, label)?);
            }
            Event::Empty(event) => {
                node_count = node_count.saturating_add(1);
                if node_count > DEFAULT_MAX_XML_NODES {
                    return Err(format!("docx_xml_node_limit:{}", label));
                }
                let node = node_from_start(&event, label)?;
                stack
                    .last_mut()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?
                    .children
                    .push(node);
            }
            Event::Text(event) => {
                let raw = String::from_utf8_lossy(event.as_ref());
                let value = unescape(raw.as_ref()).map_err(|error| {
                    format!("docx_xml_text_unescape_failed:{}:{}", label, error)
                })?;
                stack
                    .last_mut()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?
                    .text
                    .push_str(value.as_ref());
            }
            Event::CData(event) => {
                let decoded = event
                    .decode()
                    .map_err(|error| format!("docx_xml_cdata_decode_failed:{}:{}", label, error))?;
                stack
                    .last_mut()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?
                    .text
                    .push_str(decoded.as_ref());
            }
            Event::GeneralRef(event) => {
                let raw = String::from_utf8_lossy(event.as_ref());
                stack
                    .last_mut()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?
                    .text
                    .push_str(&decode_general_reference(raw.as_ref()));
            }
            Event::End(_) => {
                if stack.len() <= 1 {
                    return Err(format!("docx_xml_tree_unbalanced_end:{}", label));
                }
                let node = stack
                    .pop()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?;
                stack
                    .last_mut()
                    .ok_or_else(|| format!("docx_xml_tree_stack_underflow:{}", label))?
                    .children
                    .push(node);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if stack.len() != 1 {
        return Err(format!("docx_xml_tree_unclosed_element:{}", label));
    }
    let mut root = stack
        .pop()
        .ok_or_else(|| format!("docx_xml_tree_missing_root:{}", label))?;
    if root.children.len() != 1 {
        return Err(format!(
            "docx_xml_tree_root_count:{}:{}",
            label,
            root.children.len()
        ));
    }
    Ok(root.children.remove(0))
}

fn decode_general_reference(value: &str) -> String {
    match value {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => char::from_u32(34).unwrap().to_string(),
        "apos" => "'".to_string(),
        value if value.starts_with("#x") || value.starts_with("#X") => {
            u32::from_str_radix(&value[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|character| character.to_string())
                .unwrap_or_else(|| format!("&{value};"))
        }
        value if value.starts_with('#') => value[1..]
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|character| character.to_string())
            .unwrap_or_else(|| format!("&{value};")),
        value => format!("&{value};"),
    }
}

fn node_from_start(
    event: &quick_xml::events::BytesStart<'_>,
    label: &str,
) -> Result<XmlNode, String> {
    let mut attributes = BTreeMap::new();
    for attribute in event.attributes().with_checks(false) {
        if attributes.len() >= DEFAULT_MAX_XML_ATTRIBUTES {
            return Err(format!("docx_xml_attribute_limit:{}", label));
        }
        let attribute = attribute
            .map_err(|error| format!("docx_xml_attribute_parse_failed:{}:{}", label, error))?;
        let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .unescape_value()
            .map_err(|error| format!("docx_xml_attribute_decode_failed:{}:{}", label, error))?
            .into_owned();
        attributes.insert(key, value);
    }
    Ok(XmlNode {
        name: String::from_utf8_lossy(event.name().as_ref()).into_owned(),
        attributes,
        ..XmlNode::default()
    })
}

#[cfg(test)]
mod tests {
    use super::{local_name_of, parse_xml};

    #[test]
    fn preserves_spaces_tabs_and_entities_in_document_order() {
        let root = parse_xml(
            br#"<w:p xmlns:w="urn:w"><w:r><w:t xml:space="preserve"> A &amp; B </w:t><w:tab/><w:br/></w:r></w:p>"#,
            "test",
        )
        .unwrap();
        assert_eq!(root.local_name(), "p");
        assert_eq!(root.descendants_named("t")[0].text_content(), " A & B ");
        assert!(root.descendants_named("tab").len() == 1);
        assert_eq!(local_name_of("w:p"), "p");
    }

    trait LocalName {
        fn local_name(&self) -> &str;
    }

    impl LocalName for super::XmlNode {
        fn local_name(&self) -> &str {
            local_name_of(&self.name)
        }
    }
}
