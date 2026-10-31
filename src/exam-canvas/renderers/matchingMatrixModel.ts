// Keep this structural rendering rule in sync with PDF2Test's matchingMatrixModel.
// No runtime contract or answer serialization changes are needed for old packages.
interface Option { optionId: string; label: string; content: unknown[] }
interface Response { responseGroupId: string; kind: string; assignment: string; slotIds: string[] }

export function matchingMatrixFor<O extends Option, R extends Response>(
  task: { taskType: string; responseGroups: R[] },
  optionsFor: (response: R) => O[],
  interactionFor: (slotId: string) => string | undefined,
  paragraphMap: Record<string, string> = {}
): { options: O[]; rows: { response: R; slotId: string; interaction: 'radio' | 'checkbox' }[] } | undefined {
  const rows: { response: R; slotId: string; interaction: 'radio' | 'checkbox' }[] = []
  let options: O[] = []
  let keys: Set<string> | undefined
  const slots = new Set<string>()
  for (const response of task.responseGroups) {
    if (response.kind !== 'matching' || response.assignment !== 'per_slot' || response.slotIds.length !== 1) return
    const slotId = response.slotIds[0]
    const interaction = interactionFor(slotId)
    if ((interaction !== 'radio' && interaction !== 'checkbox') || slots.has(slotId)) return
    slots.add(slotId)
    const choices = optionsFor(response)
    if (!choices.length) return
    const currentKeys = new Set(choices.map(option => {
      // The shipped compiler used response-scoped IDs for derived paragraph
      // choices. Resolve ONLY that known generated form to its paragraph target;
      // explicit inline options still require identical optionId AND label.
      const derived = task.taskType === 'matching_information'
        && paragraphMap[option.label]
        && option.optionId === `${response.responseGroupId}-paragraph-${option.label}`
        && option.content.length === 1
        && (option.content[0] as { type?: string; text?: string })?.type === 'text'
        && (option.content[0] as { text?: string })?.text === option.label
      return JSON.stringify([derived ? `paragraph:${paragraphMap[option.label]}` : option.optionId, option.label])
    }))
    if (currentKeys.size !== choices.length || new Set(choices.map(o => o.label)).size !== choices.length) return
    if (keys && (keys.size !== currentKeys.size || [...keys].some(key => !currentKeys.has(key)))) return
    if (!keys) { keys = currentKeys; options = choices }
    rows.push({ response, slotId, interaction })
  }
  return rows.length ? { options, rows } : undefined
}
