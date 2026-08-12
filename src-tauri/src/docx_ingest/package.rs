use quick_xml::{events::Event, Reader};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use zip::{result::ZipError, ZipArchive};

const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
const ROOT_RELATIONSHIPS_PART: &str = "_rels/.rels";
const MAIN_DOCUMENT_RELATIONSHIP_SUFFIX: &str = "/officeDocument";

const DEFAULT_MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_MAX_ENTRIES: usize = 4096;
const DEFAULT_MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_XML_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_MAX_PATH_BYTES: usize = 1024;
const DEFAULT_MAX_RELATIONSHIPS: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocxPackageLimits {
    pub(crate) max_archive_bytes: u64,
    pub(crate) max_entries: usize,
    pub(crate) max_uncompressed_bytes: u64,
    pub(crate) max_entry_uncompressed_bytes: u64,
    pub(crate) max_xml_bytes: u64,
    pub(crate) max_path_bytes: usize,
    pub(crate) max_relationships: usize,
}

impl Default for DocxPackageLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: DEFAULT_MAX_ARCHIVE_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_uncompressed_bytes: DEFAULT_MAX_UNCOMPRESSED_BYTES,
            max_entry_uncompressed_bytes: DEFAULT_MAX_ENTRY_UNCOMPRESSED_BYTES,
            max_xml_bytes: DEFAULT_MAX_XML_BYTES,
            max_path_bytes: DEFAULT_MAX_PATH_BYTES,
            max_relationships: DEFAULT_MAX_RELATIONSHIPS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocxPackageEntry {
    pub(crate) path: String,
    pub(crate) compressed_size: u64,
    pub(crate) uncompressed_size: u64,
    pub(crate) is_directory: bool,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocxRelationship {
    pub(crate) source_part: String,
    pub(crate) id: String,
    pub(crate) relationship_type: String,
    pub(crate) target: String,
    pub(crate) target_mode: Option<String>,
    pub(crate) resolved_target: Option<String>,
}

impl DocxRelationship {
    pub(crate) fn is_external(&self) -> bool {
        self.target_mode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("external"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentTypes {
    defaults: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
}

impl ContentTypes {
    fn content_type(&self, part_path: &str) -> Option<&str> {
        if let Some(content_type) = self.overrides.get(part_path) {
            return Some(content_type.as_str());
        }
        let extension = part_path
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))?
            .to_ascii_lowercase();
        self.defaults.get(&extension).map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocxPackage {
    entries: BTreeMap<String, DocxPackageEntry>,
    content_types: ContentTypes,
    relationships: BTreeMap<String, Vec<DocxRelationship>>,
}

impl DocxPackage {
    pub(crate) fn part_bytes(&self, part_path: &str) -> Option<&[u8]> {
        let normalized = normalize_part_path(part_path, DEFAULT_MAX_PATH_BYTES, true).ok()?;
        self.entries
            .get(&normalized)
            .filter(|entry| !entry.is_directory)
            .map(|entry| entry.bytes.as_slice())
    }

    pub(crate) fn entry(&self, part_path: &str) -> Option<&DocxPackageEntry> {
        let normalized = normalize_part_path(part_path, DEFAULT_MAX_PATH_BYTES, true).ok()?;
        self.entries.get(&normalized)
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &DocxPackageEntry> {
        self.entries.values()
    }

    pub(crate) fn content_type(&self, part_path: &str) -> Option<&str> {
        let normalized = normalize_part_path(part_path, DEFAULT_MAX_PATH_BYTES, true).ok()?;
        self.content_types.content_type(&normalized)
    }

    pub(crate) fn relationships_for(&self, source_part: &str) -> &[DocxRelationship] {
        let normalized = if source_part.is_empty() {
            String::new()
        } else {
            match normalize_part_path(source_part, DEFAULT_MAX_PATH_BYTES, true) {
                Ok(path) => path,
                Err(_) => return &[],
            }
        };
        self.relationships
            .get(&normalized)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn relationships(&self) -> impl Iterator<Item = &DocxRelationship> {
        self.relationships.values().flatten()
    }

    pub(crate) fn main_document_part(&self) -> Option<&str> {
        self.relationships_for("")
            .iter()
            .find(|relationship| {
                !relationship.is_external()
                    && relationship
                        .relationship_type
                        .ends_with(MAIN_DOCUMENT_RELATIONSHIP_SUFFIX)
            })
            .and_then(|relationship| relationship.resolved_target.as_deref())
    }
}

pub(crate) fn is_rejected_package_error(error: &str) -> bool {
    error.contains("docx_package_rejected:")
}

pub(crate) fn open_docx(path: &Path, limits: DocxPackageLimits) -> Result<DocxPackage, String> {
    validate_docx_path(path, &limits)?;
    let mut file = fs::File::open(path)
        .map_err(|error| format!("docx_package_io:open:{}:{}", path.display(), error))?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).map_err(|error| {
        rejected(
            "DOCX_PACKAGE_BAD_MAGIC",
            format!("{}:{}", path.display(), error),
        )
    })?;
    if &magic[..2] != b"PK" {
        return Err(rejected(
            "DOCX_PACKAGE_BAD_MAGIC",
            path.display().to_string(),
        ));
    }
    let archive_bytes = file
        .metadata()
        .map_err(|error| format!("docx_package_io:metadata:{}:{}", path.display(), error))?
        .len();
    if archive_bytes > limits.max_archive_bytes {
        return Err(rejected(
            "DOCX_PACKAGE_ARCHIVE_LIMIT",
            format!("{}>{}", archive_bytes, limits.max_archive_bytes),
        ));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("docx_package_io:rewind:{}:{}", path.display(), error))?;
    read_docx_package(file, limits)
}

fn validate_docx_path(path: &Path, limits: &DocxPackageLimits) -> Result<(), String> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
    {
        return Err(rejected(
            "DOCX_PACKAGE_EXTENSION",
            path.display().to_string(),
        ));
    }
    if limits.max_archive_bytes == 0
        || limits.max_entries == 0
        || limits.max_uncompressed_bytes == 0
        || limits.max_entry_uncompressed_bytes == 0
        || limits.max_xml_bytes == 0
        || limits.max_path_bytes == 0
        || limits.max_relationships == 0
    {
        return Err(rejected(
            "DOCX_PACKAGE_INVALID_LIMITS",
            "all package limits must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn read_docx_package<R: Read + Seek>(
    reader: R,
    limits: DocxPackageLimits,
) -> Result<DocxPackage, String> {
    if limits.max_entries == 0
        || limits.max_uncompressed_bytes == 0
        || limits.max_entry_uncompressed_bytes == 0
        || limits.max_xml_bytes == 0
        || limits.max_path_bytes == 0
        || limits.max_relationships == 0
    {
        return Err(rejected(
            "DOCX_PACKAGE_INVALID_LIMITS",
            "all package limits must be greater than zero".to_string(),
        ));
    }

    let mut archive = ZipArchive::new(reader)
        .map_err(|error| rejected("DOCX_PACKAGE_ZIP_INVALID", error.to_string()))?;
    if archive.len() > limits.max_entries {
        return Err(rejected(
            "DOCX_PACKAGE_ENTRY_LIMIT",
            format!("{}>{}", archive.len(), limits.max_entries),
        ));
    }

    let mut entries = BTreeMap::new();
    let mut seen_paths = HashSet::new();
    let mut seen_casefolded_paths = HashSet::new();
    let mut total_uncompressed = 0_u64;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| match error {
            ZipError::UnsupportedArchive(message) if message == ZipError::PASSWORD_REQUIRED => {
                rejected(
                    "DOCX_PACKAGE_ENCRYPTED_ENTRY",
                    format!("entry-index-{index}"),
                )
            }
            error => rejected("DOCX_PACKAGE_ENTRY_READ", error.to_string()),
        })?;
        if file.encrypted() {
            return Err(rejected(
                "DOCX_PACKAGE_ENCRYPTED_ENTRY",
                file.name().to_string(),
            ));
        }
        if file.is_symlink() {
            return Err(rejected(
                "DOCX_PACKAGE_SYMLINK_ENTRY",
                file.name().to_string(),
            ));
        }

        let is_directory = file.is_dir();
        let path = normalize_entry_path(file.name(), is_directory, &limits)?;
        if !seen_paths.insert(path.clone()) {
            return Err(rejected("DOCX_PACKAGE_DUPLICATE_ENTRY", path));
        }
        if !seen_casefolded_paths.insert(path.to_lowercase()) {
            return Err(rejected("DOCX_PACKAGE_DUPLICATE_ENTRY", path));
        }

        let uncompressed_size = file.size();
        if uncompressed_size > limits.max_entry_uncompressed_bytes {
            return Err(rejected(
                "DOCX_PACKAGE_ENTRY_LIMIT",
                format!(
                    "{}>{}:{}",
                    uncompressed_size, limits.max_entry_uncompressed_bytes, path
                ),
            ));
        }
        if is_xml_part(&path) && uncompressed_size > limits.max_xml_bytes {
            return Err(rejected(
                "DOCX_PACKAGE_XML_LIMIT",
                format!("{}>{}:{}", uncompressed_size, limits.max_xml_bytes, path),
            ));
        }
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed_size)
            .ok_or_else(|| rejected("DOCX_PACKAGE_TOTAL_LIMIT", path.clone()))?;
        if total_uncompressed > limits.max_uncompressed_bytes {
            return Err(rejected(
                "DOCX_PACKAGE_TOTAL_LIMIT",
                format!("{}>{}", total_uncompressed, limits.max_uncompressed_bytes),
            ));
        }

        let mut bytes = Vec::new();
        if !is_directory {
            let capacity = usize::try_from(uncompressed_size).map_err(|_| {
                rejected(
                    "DOCX_PACKAGE_ENTRY_LIMIT",
                    format!("entry size does not fit in memory: {}", path),
                )
            })?;
            bytes.reserve(capacity);
            file.read_to_end(&mut bytes).map_err(|error| {
                rejected("DOCX_PACKAGE_ENTRY_READ", format!("{}:{}", path, error))
            })?;
            if bytes.len() as u64 != uncompressed_size {
                return Err(rejected(
                    "DOCX_PACKAGE_ENTRY_SIZE_MISMATCH",
                    format!("{}:{}!={}", path, bytes.len(), uncompressed_size),
                ));
            }
        }

        entries.insert(
            path.clone(),
            DocxPackageEntry {
                path,
                compressed_size: file.compressed_size(),
                uncompressed_size,
                is_directory,
                bytes,
            },
        );
    }

    let content_types_xml = entries
        .get(CONTENT_TYPES_PART)
        .filter(|entry| !entry.is_directory)
        .map(|entry| entry.bytes.as_slice())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_MISSING",
                CONTENT_TYPES_PART.to_string(),
            )
        })?;
    let content_types = parse_content_types(content_types_xml, &limits)?;
    for part_path in content_types.overrides.keys() {
        if !entries.contains_key(part_path) {
            return Err(rejected(
                "DOCX_PACKAGE_CONTENT_TYPE_TARGET_MISSING",
                part_path.clone(),
            ));
        }
    }

    let relationship_part_paths = entries
        .keys()
        .filter(|path| is_relationship_part(path))
        .cloned()
        .collect::<Vec<_>>();
    let mut relationships = BTreeMap::<String, Vec<DocxRelationship>>::new();
    let mut relationship_count = 0_usize;
    for relationship_part_path in relationship_part_paths {
        let source_part = relationship_source_part(&relationship_part_path)?;
        if !source_part.is_empty() && !entries.contains_key(&source_part) {
            return Err(rejected(
                "DOCX_PACKAGE_RELATIONSHIP_SOURCE_MISSING",
                format!("{}:{}", relationship_part_path, source_part),
            ));
        }
        let xml = entries
            .get(&relationship_part_path)
            .filter(|entry| !entry.is_directory)
            .map(|entry| entry.bytes.as_slice())
            .ok_or_else(|| {
                rejected(
                    "DOCX_PACKAGE_RELATIONSHIP_PART_INVALID",
                    relationship_part_path.clone(),
                )
            })?;
        let parsed = parse_relationships(&source_part, xml, &limits, &entries)?;
        relationship_count = relationship_count
            .checked_add(parsed.len())
            .ok_or_else(|| {
                rejected(
                    "DOCX_PACKAGE_RELATIONSHIP_LIMIT",
                    relationship_part_path.clone(),
                )
            })?;
        if relationship_count > limits.max_relationships {
            return Err(rejected(
                "DOCX_PACKAGE_RELATIONSHIP_LIMIT",
                format!("{}>{}", relationship_count, limits.max_relationships),
            ));
        }
        relationships.insert(source_part, parsed);
    }

    let has_main_document_part = relationships
        .get("")
        .into_iter()
        .flatten()
        .find(|relationship| {
            !relationship.is_external()
                && relationship
                    .relationship_type
                    .ends_with(MAIN_DOCUMENT_RELATIONSHIP_SUFFIX)
        })
        .and_then(|relationship| relationship.resolved_target.as_deref())
        .map(|part_path| {
            entries
                .get(part_path)
                .is_some_and(|entry| !entry.is_directory)
        })
        .unwrap_or_else(|| {
            entries
                .get("word/document.xml")
                .is_some_and(|entry| !entry.is_directory)
        });
    if !has_main_document_part {
        return Err(rejected(
            "DOCX_PACKAGE_DOCUMENT_MISSING",
            "main document relationship or word/document.xml".to_string(),
        ));
    }

    Ok(DocxPackage {
        entries,
        content_types,
        relationships,
    })
}

fn is_xml_part(path: &str) -> bool {
    path.eq_ignore_ascii_case(CONTENT_TYPES_PART)
        || path.rsplit_once('.').is_some_and(|(_, extension)| {
            matches!(extension.to_ascii_lowercase().as_str(), "xml" | "rels")
        })
}

fn normalize_entry_path(
    raw_path: &str,
    is_directory: bool,
    limits: &DocxPackageLimits,
) -> Result<String, String> {
    let path = if is_directory {
        raw_path
            .strip_suffix('/')
            .ok_or_else(|| rejected("DOCX_PACKAGE_DIRECTORY_PATH", raw_path.to_string()))?
    } else {
        raw_path
    };
    normalize_part_path(path, limits.max_path_bytes, false).map_err(|error| {
        rejected(
            "DOCX_PACKAGE_PATH_UNSAFE",
            format!("{}:{}", raw_path, error),
        )
    })
}

fn normalize_part_path(
    raw_path: &str,
    max_path_bytes: usize,
    allow_leading_slash: bool,
) -> Result<String, String> {
    if raw_path.is_empty() || raw_path.len() > max_path_bytes {
        return Err("empty or oversized path".to_string());
    }
    if raw_path.contains('\0') || raw_path.contains('\\') {
        return Err("NUL or backslash is not allowed".to_string());
    }
    let path = if allow_leading_slash {
        raw_path.strip_prefix('/').unwrap_or(raw_path)
    } else {
        if raw_path.starts_with('/') {
            return Err("absolute path is not allowed".to_string());
        }
        raw_path
    };
    if path.is_empty() || path.ends_with('/') {
        return Err("empty path component".to_string());
    }

    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err("empty, current, or parent path component".to_string());
        }
        if component.chars().any(|character| {
            character.is_control() || matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|')
        }) {
            return Err("unsupported path character".to_string());
        }
    }
    Ok(path.to_string())
}

fn parse_content_types(xml: &[u8], limits: &DocxPackageLimits) -> Result<ContentTypes, String> {
    ensure_xml_size(xml, limits)?;
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut root_seen = false;
    let mut depth = 0_u32;
    let mut defaults = BTreeMap::new();
    let mut overrides = BTreeMap::new();

    loop {
        match reader.read_event_into(&mut buffer).map_err(|error| {
            rejected("DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID", error.to_string())
        })? {
            Event::Start(event) => {
                let event_name = event.name();
                let name = local_name(event_name.as_ref());
                if !root_seen {
                    if name != b"Types" {
                        return Err(rejected(
                            "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                            "root element must be Types".to_string(),
                        ));
                    }
                    root_seen = true;
                }
                depth = depth.saturating_add(1);
                if name == b"Default" {
                    insert_default_content_type(&mut defaults, &event)?;
                } else if name == b"Override" {
                    insert_override_content_type(&mut overrides, &event, limits)?;
                }
            }
            Event::Empty(event) => {
                let event_name = event.name();
                let name = local_name(event_name.as_ref());
                if !root_seen && name != b"Types" {
                    return Err(rejected(
                        "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                        "root element must be Types".to_string(),
                    ));
                }
                if name == b"Types" {
                    root_seen = true;
                } else if name == b"Default" {
                    insert_default_content_type(&mut defaults, &event)?;
                } else if name == b"Override" {
                    insert_override_content_type(&mut overrides, &event, limits)?;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    rejected(
                        "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                        "unexpected closing element".to_string(),
                    )
                })?;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root_seen || depth != 0 {
        return Err(rejected(
            "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
            "missing or unclosed Types root".to_string(),
        ));
    }
    Ok(ContentTypes {
        defaults,
        overrides,
    })
}

fn insert_default_content_type(
    defaults: &mut BTreeMap<String, String>,
    event: &quick_xml::events::BytesStart<'_>,
) -> Result<(), String> {
    let extension = attribute(event, b"Extension")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                "Default.Extension".to_string(),
            )
        })?
        .to_ascii_lowercase();
    if extension.contains('/') || extension.contains('\\') {
        return Err(rejected(
            "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
            format!("invalid extension: {}", extension),
        ));
    }
    let content_type = attribute(event, b"ContentType")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                "Default.ContentType".to_string(),
            )
        })?;
    if defaults.insert(extension.clone(), content_type).is_some() {
        return Err(rejected(
            "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
            format!("duplicate default extension: {}", extension),
        ));
    }
    Ok(())
}

fn insert_override_content_type(
    overrides: &mut BTreeMap<String, String>,
    event: &quick_xml::events::BytesStart<'_>,
    limits: &DocxPackageLimits,
) -> Result<(), String> {
    let part_name = attribute(event, b"PartName")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                "Override.PartName".to_string(),
            )
        })?;
    let part_path =
        normalize_part_path(&part_name, limits.max_path_bytes, true).map_err(|error| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                format!("{}:{}", part_name, error),
            )
        })?;
    let content_type = attribute(event, b"ContentType")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
                "Override.ContentType".to_string(),
            )
        })?;
    if overrides.insert(part_path.clone(), content_type).is_some() {
        return Err(rejected(
            "DOCX_PACKAGE_CONTENT_TYPES_XML_INVALID",
            format!("duplicate override part: {}", part_path),
        ));
    }
    Ok(())
}

fn is_relationship_part(path: &str) -> bool {
    path == ROOT_RELATIONSHIPS_PART || (path.ends_with(".rels") && path.contains("/_rels/"))
}

fn relationship_source_part(relationship_part: &str) -> Result<String, String> {
    if relationship_part == ROOT_RELATIONSHIPS_PART {
        return Ok(String::new());
    }
    let (directory, file_name) = relationship_part.rsplit_once('/').ok_or_else(|| {
        rejected(
            "DOCX_PACKAGE_RELATIONSHIP_PART_INVALID",
            relationship_part.to_string(),
        )
    })?;
    if !directory.ends_with("/_rels") || !file_name.ends_with(".rels") {
        return Err(rejected(
            "DOCX_PACKAGE_RELATIONSHIP_PART_INVALID",
            relationship_part.to_string(),
        ));
    }
    let source_name = file_name
        .strip_suffix(".rels")
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_RELATIONSHIP_PART_INVALID",
                relationship_part.to_string(),
            )
        })?;
    let source_directory = directory.strip_suffix("/_rels").unwrap_or("");
    if source_directory.is_empty() {
        normalize_part_path(source_name, DEFAULT_MAX_PATH_BYTES, false)
            .map_err(|error| rejected("DOCX_PACKAGE_RELATIONSHIP_PART_INVALID", error))
    } else {
        normalize_part_path(
            &format!("{}/{}", source_directory, source_name),
            DEFAULT_MAX_PATH_BYTES,
            false,
        )
        .map_err(|error| rejected("DOCX_PACKAGE_RELATIONSHIP_PART_INVALID", error))
    }
}

fn parse_relationships(
    source_part: &str,
    xml: &[u8],
    limits: &DocxPackageLimits,
    entries: &BTreeMap<String, DocxPackageEntry>,
) -> Result<Vec<DocxRelationship>, String> {
    ensure_xml_size(xml, limits)?;
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut root_seen = false;
    let mut depth = 0_u32;
    let mut ids = HashSet::new();
    let mut relationships = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer).map_err(|error| {
            rejected("DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID", error.to_string())
        })? {
            Event::Start(event) => {
                let event_name = event.name();
                let name = local_name(event_name.as_ref());
                if !root_seen {
                    if name != b"Relationships" {
                        return Err(rejected(
                            "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                            format!("{}:root element must be Relationships", source_part),
                        ));
                    }
                    root_seen = true;
                }
                depth = depth.saturating_add(1);
                if name == b"Relationship" {
                    relationships.push(parse_relationship(source_part, &event, &mut ids, entries)?);
                }
            }
            Event::Empty(event) => {
                let event_name = event.name();
                let name = local_name(event_name.as_ref());
                if !root_seen && name != b"Relationships" {
                    return Err(rejected(
                        "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                        format!("{}:root element must be Relationships", source_part),
                    ));
                }
                if name == b"Relationships" {
                    root_seen = true;
                } else if name == b"Relationship" {
                    relationships.push(parse_relationship(source_part, &event, &mut ids, entries)?);
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    rejected(
                        "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                        format!("{}:unexpected closing element", source_part),
                    )
                })?;
            }
            Event::Eof => break,
            _ => {}
        }
        if relationships.len() > limits.max_relationships {
            return Err(rejected(
                "DOCX_PACKAGE_RELATIONSHIP_LIMIT",
                format!(
                    "{}>{}:{}",
                    relationships.len(),
                    limits.max_relationships,
                    source_part
                ),
            ));
        }
        buffer.clear();
    }
    if !root_seen || depth != 0 {
        return Err(rejected(
            "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
            format!("{}:missing or unclosed Relationships root", source_part),
        ));
    }
    Ok(relationships)
}

fn parse_relationship(
    source_part: &str,
    event: &quick_xml::events::BytesStart<'_>,
    ids: &mut HashSet<String>,
    entries: &BTreeMap<String, DocxPackageEntry>,
) -> Result<DocxRelationship, String> {
    let id = attribute(event, b"Id")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                "Relationship.Id".to_string(),
            )
        })?;
    if !ids.insert(id.clone()) {
        return Err(rejected(
            "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
            format!("{}:duplicate relationship id {}", source_part, id),
        ));
    }
    let relationship_type = attribute(event, b"Type")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                "Relationship.Type".to_string(),
            )
        })?;
    let target = attribute(event, b"Target")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            rejected(
                "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                "Relationship.Target".to_string(),
            )
        })?;
    if target
        .chars()
        .any(|character| character.is_control() || character == '\0')
    {
        return Err(rejected(
            "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
            format!(
                "{}:relationship target contains a control character",
                source_part
            ),
        ));
    }
    let target_mode = attribute(event, b"TargetMode");
    let is_external = target_mode
        .as_deref()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("external"));
    if let Some(mode) = target_mode.as_deref() {
        if !is_external && !mode.eq_ignore_ascii_case("internal") {
            return Err(rejected(
                "DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID",
                format!("{}:unsupported TargetMode {}", source_part, mode),
            ));
        }
    }

    let resolved_target = if is_external {
        None
    } else {
        let resolved = resolve_internal_target(source_part, &target)?;
        if !entries.contains_key(&resolved) {
            return Err(rejected(
                "DOCX_PACKAGE_RELATIONSHIP_TARGET_MISSING",
                format!("{}:{}", source_part, resolved),
            ));
        }
        Some(resolved)
    };

    Ok(DocxRelationship {
        source_part: source_part.to_string(),
        id,
        relationship_type,
        target,
        target_mode,
        resolved_target,
    })
}

fn resolve_internal_target(source_part: &str, target: &str) -> Result<String, String> {
    if target.is_empty() || target.contains('\\') || target.starts_with("//") {
        return Err(rejected(
            "DOCX_PACKAGE_RELATIONSHIP_TARGET_UNSAFE",
            format!("{}:{}", source_part, target),
        ));
    }
    let mut components = if target.starts_with('/') {
        Vec::new()
    } else if source_part.is_empty() {
        Vec::new()
    } else {
        source_part
            .rsplit_once('/')
            .map(|(directory, _)| directory.split('/').map(str::to_string).collect())
            .unwrap_or_default()
    };
    let target_path = target.strip_prefix('/').unwrap_or(target);
    for component in target_path.split('/') {
        if component.is_empty() || component == "." {
            if component.is_empty() {
                return Err(rejected(
                    "DOCX_PACKAGE_RELATIONSHIP_TARGET_UNSAFE",
                    format!("{}:{}", source_part, target),
                ));
            }
            continue;
        }
        if component == ".." {
            if components.pop().is_none() {
                return Err(rejected(
                    "DOCX_PACKAGE_RELATIONSHIP_TARGET_UNSAFE",
                    format!("{}:{}", source_part, target),
                ));
            }
            continue;
        }
        components.push(component.to_string());
    }
    let resolved = components.join("/");
    normalize_part_path(&resolved, DEFAULT_MAX_PATH_BYTES, false).map_err(|error| {
        rejected(
            "DOCX_PACKAGE_RELATIONSHIP_TARGET_UNSAFE",
            format!("{}:{}:{}", source_part, target, error),
        )
    })
}

fn ensure_xml_size(xml: &[u8], limits: &DocxPackageLimits) -> Result<(), String> {
    if xml.len() as u64 > limits.max_xml_bytes {
        return Err(rejected(
            "DOCX_PACKAGE_XML_LIMIT",
            format!("{}>{}", xml.len(), limits.max_xml_bytes),
        ));
    }
    Ok(())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn attribute(event: &quick_xml::events::BytesStart<'_>, wanted: &[u8]) -> Option<String> {
    event
        .attributes()
        .filter_map(Result::ok)
        .find(|attribute| local_name(attribute.key.as_ref()) == wanted)
        .and_then(|attribute| String::from_utf8(attribute.value.as_ref().to_vec()).ok())
}

fn rejected(code: &str, detail: String) -> String {
    format!("docx_package_rejected:{}:{}", code, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn options() -> SimpleFileOptions {
        SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
    }

    fn package_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        for (path, bytes) in entries {
            writer.start_file(*path, options()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        output.into_inner()
    }

    fn valid_entries() -> Vec<(&'static str, &'static [u8])> {
        vec![
            (
                CONTENT_TYPES_PART,
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/media/image1.png" ContentType="image/png"/></Types>"#,
            ),
            (
                ROOT_RELATIONSHIPS_PART,
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rIdExternal" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.test/" TargetMode="External"/></Relationships>"#,
            ),
            (
                "word/document.xml",
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rIdUp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/></Relationships>"#,
            ),
            ("word/media/image1.png", b"png"),
            ("customXml/item1.xml", b"<item/>"),
        ]
    }

    fn read_bytes(bytes: Vec<u8>, limits: DocxPackageLimits) -> Result<DocxPackage, String> {
        read_docx_package(Cursor::new(bytes), limits)
    }

    #[test]
    fn reads_content_types_and_resolves_internal_relationships_without_fetching_external_targets() {
        let package = read_bytes(
            package_bytes(&valid_entries()),
            DocxPackageLimits::default(),
        )
        .expect("valid package should open");

        assert_eq!(package.main_document_part(), Some("word/document.xml"));
        assert_eq!(
            package.content_type("word/document.xml"),
            Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
            )
        );
        assert_eq!(
            package.content_type("word/media/image1.png"),
            Some("image/png")
        );
        assert_eq!(package.part_bytes("/word/document.xml").unwrap().len(), 84);

        let root_relationships = package.relationships_for("");
        assert_eq!(root_relationships.len(), 2);
        assert_eq!(
            root_relationships
                .iter()
                .find(|relationship| relationship.id == "rIdExternal")
                .and_then(|relationship| relationship.resolved_target.as_deref()),
            None
        );

        let document_relationships = package.relationships_for("word/document.xml");
        assert_eq!(document_relationships.len(), 2);
        assert_eq!(
            document_relationships
                .iter()
                .find(|relationship| relationship.id == "rIdImage")
                .and_then(|relationship| relationship.resolved_target.as_deref()),
            Some("word/media/image1.png")
        );
        assert_eq!(
            document_relationships
                .iter()
                .find(|relationship| relationship.id == "rIdUp")
                .and_then(|relationship| relationship.resolved_target.as_deref()),
            Some("customXml/item1.xml")
        );
    }

    #[test]
    fn rejects_zip_slip_absolute_and_windows_path_entries() {
        for unsafe_path in [
            "../escape.xml",
            "/absolute.xml",
            "C:/drive.xml",
            "word\\document.xml",
        ] {
            let mut entries = valid_entries();
            entries.push((unsafe_path, b"bad"));
            let error = read_bytes(package_bytes(&entries), DocxPackageLimits::default())
                .expect_err("unsafe package path should be rejected");
            assert!(
                error.contains("DOCX_PACKAGE_PATH_UNSAFE"),
                "unexpected error for {unsafe_path}: {error}"
            );
            assert!(is_rejected_package_error(&error));
        }
    }

    #[test]
    fn rejects_symlink_entries() {
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        writer
            .add_symlink("word/link", "../outside.xml", options())
            .unwrap();
        writer.finish().unwrap();

        let error = read_bytes(output.into_inner(), DocxPackageLimits::default())
            .expect_err("symlink entry should be rejected");
        assert!(error.contains("DOCX_PACKAGE_SYMLINK_ENTRY"));
    }

    #[test]
    fn rejects_duplicate_paths_and_broken_internal_relationships() {
        let mut duplicate_entries = valid_entries();
        duplicate_entries.push(("WORD/DOCUMENT.XML", b"duplicate"));
        let duplicate_error = read_bytes(
            package_bytes(&duplicate_entries),
            DocxPackageLimits::default(),
        )
        .expect_err("duplicate entry should be rejected");
        assert!(duplicate_error.contains("DOCX_PACKAGE_DUPLICATE_ENTRY"));

        let mut broken_entries = valid_entries();
        broken_entries[3] = (
            "word/_rels/document.xml.rels",
            br#"<Relationships><Relationship Id="rId1" Type="urn:test" Target="../../escape.xml"/></Relationships>"#,
        );
        let broken_error = read_bytes(package_bytes(&broken_entries), DocxPackageLimits::default())
            .expect_err("relationship escaping package root should be rejected");
        assert!(broken_error.contains("DOCX_PACKAGE_RELATIONSHIP_TARGET_UNSAFE"));
    }

    #[test]
    fn enforces_entry_count_and_total_uncompressed_limits_before_accepting_payload() {
        let mut entry_limit = DocxPackageLimits::default();
        entry_limit.max_entries = 2;
        let entry_error = read_bytes(package_bytes(&valid_entries()), entry_limit)
            .expect_err("entry count should be bounded");
        assert!(entry_error.contains("DOCX_PACKAGE_ENTRY_LIMIT"));

        let mut total_limit = DocxPackageLimits::default();
        total_limit.max_uncompressed_bytes = 128;
        let total_error = read_bytes(package_bytes(&valid_entries()), total_limit)
            .expect_err("total uncompressed bytes should be bounded");
        assert!(total_error.contains("DOCX_PACKAGE_TOTAL_LIMIT"));

        let mut entry_size_limit = DocxPackageLimits::default();
        entry_size_limit.max_entry_uncompressed_bytes = 32;
        let entry_size_error = read_bytes(package_bytes(&valid_entries()), entry_size_limit)
            .expect_err("single entry size should be bounded");
        assert!(entry_size_error.contains("DOCX_PACKAGE_ENTRY_LIMIT"));
    }

    #[test]
    fn rejects_missing_content_types_document_and_malformed_relationships() {
        let no_content_types = package_bytes(&[("word/document.xml", b"<w:document/>")]);
        let error = read_bytes(no_content_types, DocxPackageLimits::default())
            .expect_err("content types part is required");
        assert!(error.contains("DOCX_PACKAGE_CONTENT_TYPES_MISSING"));

        let mut malformed = valid_entries();
        malformed[1] = (ROOT_RELATIONSHIPS_PART, b"<Relationships>");
        let error = read_bytes(package_bytes(&malformed), DocxPackageLimits::default())
            .expect_err("malformed relationships should be rejected");
        assert!(error.contains("DOCX_PACKAGE_RELATIONSHIPS_XML_INVALID"));

        let missing_document = package_bytes(&[
            (CONTENT_TYPES_PART, b"<Types/>"),
            ("word/styles.xml", b"<w:styles/>"),
        ]);
        let error = read_bytes(missing_document, DocxPackageLimits::default())
            .expect_err("document part is required");
        assert!(error.contains("DOCX_PACKAGE_DOCUMENT_MISSING"));
    }

    #[test]
    fn accepts_minimal_docx_without_root_relationship_part() {
        let bytes = package_bytes(&[
            (
                CONTENT_TYPES_PART,
                br#"<Types><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/xml"/></Types>"#,
            ),
            ("word/document.xml", b"<w:document/>"),
        ]);
        let package = read_bytes(bytes, DocxPackageLimits::default())
            .expect("package reader should support the existing minimal fixture shape");
        assert!(package.relationships_for("").is_empty());
        assert_eq!(
            package.part_bytes("word/document.xml"),
            Some(b"<w:document/>".as_slice())
        );
    }

    #[test]
    fn accepts_root_relationship_selected_nonstandard_main_document_part() {
        let bytes = package_bytes(&[
            (
                CONTENT_TYPES_PART,
                br#"<Types><Default Extension="xml" ContentType="application/xml"/><Override PartName="/custom/document.xml" ContentType="application/xml"/></Types>"#,
            ),
            (
                ROOT_RELATIONSHIPS_PART,
                br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="custom/document.xml"/></Relationships>"#,
            ),
            ("custom/document.xml", b"<w:document/>"),
        ]);
        let package = read_bytes(bytes, DocxPackageLimits::default())
            .expect("root relationship should select the main document part");
        assert_eq!(package.main_document_part(), Some("custom/document.xml"));
        assert_eq!(
            package.part_bytes("custom/document.xml"),
            Some(b"<w:document/>".as_slice())
        );
    }

    #[test]
    fn open_docx_validates_extension_magic_archive_size_and_rewinds_before_zip_read() {
        let root = std::env::temp_dir();
        let stem = format!(
            "docx-package-open-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let docx_path = root.join(format!("{}.docx", stem));
        let wrong_extension_path = root.join(format!("{}.zip", stem));
        let bad_magic_path = root.join(format!("{}-bad.docx", stem));

        let bytes = package_bytes(&[
            (
                CONTENT_TYPES_PART,
                br#"<Types><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/xml"/></Types>"#,
            ),
            ("word/document.xml", b"<w:document/>"),
        ]);
        std::fs::write(&docx_path, &bytes).unwrap();
        std::fs::write(&wrong_extension_path, &bytes).unwrap();
        std::fs::write(&bad_magic_path, b"not a zip").unwrap();

        let valid = open_docx(&docx_path, DocxPackageLimits::default())
            .expect("open_docx should rewind after magic validation");
        assert_eq!(
            valid.part_bytes("word/document.xml"),
            Some(b"<w:document/>".as_slice())
        );

        let extension_error = open_docx(&wrong_extension_path, DocxPackageLimits::default())
            .expect_err("non-docx extension should be rejected");
        assert!(extension_error.contains("DOCX_PACKAGE_EXTENSION"));

        let magic_error = open_docx(&bad_magic_path, DocxPackageLimits::default())
            .expect_err("bad magic should be rejected");
        assert!(magic_error.contains("DOCX_PACKAGE_BAD_MAGIC"));

        let mut archive_limit = DocxPackageLimits::default();
        archive_limit.max_archive_bytes = bytes.len() as u64 - 1;
        let archive_error = open_docx(&docx_path, archive_limit)
            .expect_err("physical archive size should be bounded");
        assert!(archive_error.contains("DOCX_PACKAGE_ARCHIVE_LIMIT"));

        let _ = std::fs::remove_file(docx_path);
        let _ = std::fs::remove_file(wrong_extension_path);
        let _ = std::fs::remove_file(bad_magic_path);
    }
}
