// Source: IELTS-NASfor-WenDao apps/student-exam/src/modules/reading-engine/highlightDom.ts
// Keep in-place text wrapping synchronized with the NAS student renderer.
export type ReadingHighlightKind = 'highlight' | 'note'

export interface WrapHighlightOptions {
  kind: ReadingHighlightKind
  id?: string | null
  noteText?: string | null
}

export interface WrapHighlightResult {
  applied: boolean
  text: string
  elements: HTMLElement[]
}

function setHighlightAttributes(element: HTMLElement, options: WrapHighlightOptions): void {
  element.classList.add('hl')
  element.dataset.kind = options.kind
  element.dataset.hlType = options.kind
  if (options.id) {
    element.dataset.id = options.id
  }
  if (options.noteText) {
    element.dataset.note = options.noteText
  }
}

function textNodeCanBeWrapped(node: Text): boolean {
  if (!String(node.nodeValue || '').trim()) return false
  const parent = node.parentElement
  if (!parent) return true
  return !parent.closest('script, style, input, textarea, select, option, button, [contenteditable="true"], [draggable="true"], [role="button"]')
}

export function wrapRangeTextWithHighlight(range: Range, options: WrapHighlightOptions): WrapHighlightResult {
  const text = String(range.toString() || '').trim()
  if (!text || range.collapsed) {
    return { applied: false, text, elements: [] }
  }

  const documentRef = range.commonAncestorContainer.ownerDocument || document
  // Wrap selected text in place: never detach live answer controls or their bindings.
  const root = range.commonAncestorContainer
  const walker = documentRef.createTreeWalker(root, NodeFilter.SHOW_TEXT)
  const nodes: Text[] = root.nodeType === Node.TEXT_NODE ? [root as Text] : []
  while (walker.nextNode()) nodes.push(walker.currentNode as Text)
  const segments = nodes.filter((node) => textNodeCanBeWrapped(node) && range.intersectsNode(node)).map((node) => ({
    node,
    start: node === range.startContainer ? range.startOffset : 0,
    end: node === range.endContainer ? range.endOffset : node.length
  }))
  const elements: HTMLElement[] = []
  segments.forEach(({ node, start, end }) => {
    if (end <= start) return
    if (end < node.length) node.splitText(end)
    const selected = start > 0 ? node.splitText(start) : node
    const span = documentRef.createElement('span')
    setHighlightAttributes(span, options)
    selected.parentNode!.insertBefore(span, selected)
    span.appendChild(selected)
    elements.push(span)
  })
  return { applied: elements.length > 0, text, elements }
}
