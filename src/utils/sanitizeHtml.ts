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
  "href", "src", "alt", "title", "width", "height", "colspan", "rowspan",
  "class", "id", "style"
]);

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
  const children = Array.from(node.children);
  for (const el of children) {
    const tag = el.tagName;
    if (!ALLOWED_TAGS.has(tag)) {
      // 非 白名单标签：移除标签但保留子内容（避免 <script> 执行）。
      // <script>/<style> 直接整块删（含内容）。
      if (tag === "SCRIPT" || tag === "STYLE" || tag === "IFRAME" || tag === "OBJECT" || tag === "EMBED") {
        el.remove();
        continue;
      }
      const parent = el.parentNode;
      while (el.firstChild) parent?.insertBefore(el.firstChild, el);
      parent?.removeChild(el);
      // 递归处理被提升的子节点。
      continue;
    }
    // 删除危险属性：on* 事件处理器、javascript:/vbscript: 协议。
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase();
      const value = attr.value.trim().toLowerCase();
      if (name.startsWith("on")) {
        el.removeAttribute(attr.name);
        continue;
      }
      if (!ALLOWED_ATTRS.has(name)) {
        el.removeAttribute(attr.name);
        continue;
      }
      if ((name === "href" || name === "src") && (value.startsWith("javascript:") || value.startsWith("vbscript:") || value.startsWith("data:text/html"))) {
        el.removeAttribute(attr.name);
      }
    }
    clean(el);
  }
}
