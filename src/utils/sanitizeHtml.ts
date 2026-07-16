// 最小化 HTML 净化：剥离 <script>、事件处理器（on*）、javascript:/data: 协议等 XSS 载荷。
// 不引入 DOMPurify 等第三方依赖，用 DOMParser 做白名单清理。
// passage htmlBlocks 由作者工具生成，但 update_authoring_ir 全量接收前端 ir，
// 故渲染前必须净化（存储型 XSS 防御）。

const ALLOWED_TAGS = new Set([
  "P", "BR", "HR", "B", "I", "EM", "STRONG", "U", "SPAN", "DIV",
  "UL", "OL", "LI", "H1", "H2", "H3", "H4", "H5", "H6",
  "TABLE", "THEAD", "TBODY", "TR", "TD", "TH", "CAPTION", "COLGROUP", "COL",
  "IMG", "A", "BLOCKQUOTE", "PRE", "CODE", "SUB", "SUP", "MARK", "SMALL"
]);

const ALLOWED_ATTRS = new Set([
  "href", "src", "alt", "title", "width", "height", "colspan", "rowspan"
]);

const DROP_ENTIRE_TAGS = new Set([
  "SCRIPT", "STYLE", "IFRAME", "OBJECT", "EMBED", "LINK", "META", "NOSCRIPT", "TEMPLATE"
]);

const CONTROL_OR_SPACE = /[\u0000-\u0020\u007f]+/g;
const SCHEME_PREFIX = /^[a-zA-Z][a-zA-Z\d+.-]*:/;

/** 净化一段 HTML，返回安全字符串。在浏览器环境用 DOMParser；非浏览器原样返回（后端已转义）。 */
export function sanitizeHtml(html: string): string {
  if (typeof window === "undefined" || typeof DOMParser === "undefined") {
    return html;
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  clean(doc.body);
  return doc.body.innerHTML;
}

function clean(node: Element): void {
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === Node.COMMENT_NODE) {
      child.parentNode?.removeChild(child);
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) continue;

    const el = child as Element;
    const tag = el.tagName;
    if (!ALLOWED_TAGS.has(tag)) {
      if (DROP_ENTIRE_TAGS.has(tag)) {
        el.remove();
        continue;
      }
      const parent = el.parentNode;
      while (el.firstChild) parent?.insertBefore(el.firstChild, el);
      parent?.removeChild(el);
      continue;
    }

    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") || !ALLOWED_ATTRS.has(name)) {
        el.removeAttribute(attr.name);
        continue;
      }
      if (name === "href" || name === "src") {
        const safeValue = sanitizeUrlAttribute(name, attr.value);
        if (!safeValue) {
          el.removeAttribute(attr.name);
        } else {
          el.setAttribute(attr.name, safeValue);
        }
      }
    }
    clean(el);
  }
}

function sanitizeUrlAttribute(name: "href" | "src", value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  const protocolProbe = trimmed.replace(CONTROL_OR_SPACE, "");
  if (!protocolProbe || protocolProbe.startsWith("//")) return null;
  if (!SCHEME_PREFIX.test(protocolProbe)) return trimmed;

  const scheme = protocolProbe.slice(0, protocolProbe.indexOf(":") + 1).toLowerCase();
  if (name === "href") {
    return scheme === "http:" || scheme === "https:" || scheme === "mailto:" || scheme === "tel:"
      ? trimmed
      : null;
  }
  return scheme === "http:" || scheme === "https:" ? trimmed : null;
}
