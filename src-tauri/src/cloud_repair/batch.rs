//! Compact model decisions; document selection, versions and audit bindings stay local.
use super::*;

pub(super) fn execute(
    request: &RepairRunRequest<'_>,
    call: &CloudRepairToolCallV1,
    round: u32,
    context: &Value,
    mut packet: Option<&mut PacketTools<'_>>,
) -> (CloudRepairToolResultV1, Option<usize>) {
    let reject = |error: String| {
        (
            CloudRepairToolResultV1::rejected(&call.call_id, vec![error]),
            None,
        )
    };
    let Some(decisions) = call.arguments["decisions"]
        .as_array()
        .filter(|d| !d.is_empty())
    else {
        return reject("CLOUD_BATCH_DECISIONS_REQUIRED".into());
    };
    let differences = context["differences"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut rulings = Vec::new();
    let mut commands = Vec::new();
    let mut evidence = Vec::new();
    let cloud_current = context["comparisonMode"] == "adopted_cloud_vs_local_snapshot";
    for original in decisions {
        let mut expanded = original.clone();
        if let Some(alias) = original["decisionId"].as_str() {
            let resolved = alias
                .strip_prefix('d')
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| differences.get(index));
            let Some(delta) = resolved else {
                return reject(format!("CLOUD_BATCH_UNKNOWN_DECISION_ID:{alias}"));
            };
            for field in ["targetType", "targetId", "field"] {
                if original
                    .get(field)
                    .is_some_and(|value| value != &delta[field])
                {
                    return reject(format!(
                        "CLOUD_BATCH_DECISION_ID_TARGET_MISMATCH:{alias}:{field}"
                    ));
                }
                expanded[field] = delta[field].clone();
            }
        }
        if let Some(ids) = original["evidenceLineIds"].as_array() {
            let mut resolved = original["evidence"].as_array().cloned().unwrap_or_default();
            for id in ids {
                let Some(id) = id.as_str() else {
                    return reject("CLOUD_BATCH_EVIDENCE_LINE_ID_INVALID".into());
                };
                match source_line_evidence(context, id) {
                    Ok(evidence) => resolved.push(evidence),
                    Err(error) => return reject(error),
                }
            }
            expanded["evidence"] = json!(resolved);
        }
        let entry = &expanded;
        let key = difference_key(entry);
        if !differences.iter().any(|d| difference_key(d) == key) {
            return reject(format!(
                "CLOUD_BATCH_UNKNOWN_TARGET:{}:{}:{}",
                key.0, key.1, key.2
            ));
        }
        if !seen.insert(key) {
            return reject("CLOUD_BATCH_DUPLICATE_TARGET".into());
        }
        let mut ruling = entry.clone();
        let choice = entry["choice"].as_str().unwrap_or("");
        if matches!(choice, "Cloud" | "Local" | "Wrong")
            && !entry["evidence"].as_array().is_some_and(|e| !e.is_empty())
        {
            return reject("CLOUD_DECISION_SOURCE_EVIDENCE_REQUIRED".into());
        }
        ruling["decision"] = json!(match choice {
            "Cloud" =>
                if cloud_current {
                    "keep_current"
                } else {
                    "use_cloud"
                },
            "Local" =>
                if cloud_current {
                    "use_local"
                } else {
                    "keep_current"
                },
            "Unknown" => "need_context",
            "Wrong" => {
                let Some(edits) = entry["commands"].as_array().filter(|c| !c.is_empty()) else {
                    return reject("CLOUD_BATCH_WRONG_COMMANDS_REQUIRED".into());
                };
                commands.extend(edits.iter().cloned());
                evidence.extend(entry["evidence"].as_array().into_iter().flatten().cloned());
                continue;
            }
            _ => return reject(format!("CLOUD_BATCH_UNKNOWN_CHOICE:{choice}")),
        });
        rulings.push(ruling);
    }
    // Reject mixed selections for a dependency closure before any mutation. Wrong commands
    // continue through the existing scoped edit and source-evidence validators.
    // Validate corrections and every supplied quote before a candidate choice can write.
    if !commands.is_empty() {
        if evidence.is_empty() {
            return reject("CLOUD_BATCH_WRONG_EVIDENCE_REQUIRED".into());
        }
        if let Err(error) = tools::sanitize_commands(&commands) {
            return reject(error);
        }
        let Some((current, _)) = current_canonical(request).ok().flatten() else {
            return reject("ITEM_DS_NOT_SEEDED".into());
        };
        let mut allowed = BTreeSet::new();
        for group in context
            .pointer("/draftSlice/taskGroups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            allowed.extend(
                crate::library::repository::EditFootprint::for_command(
                    &current,
                    &json!({"op":"upsertTaskGroupBundle","taskGroup":group}),
                )
                .targets,
            );
        }
        if differences.iter().any(|d| d["targetType"] == "passage") {
            allowed.extend(
                crate::library::repository::EditFootprint::for_command(
                    &current,
                    &json!({"op":"replaceContent","target":{"kind":"passage"},"content":[]}),
                )
                .targets,
            );
        }
        fn existing_ids(value: &Value, ids: &mut BTreeSet<String>) {
            match value {
                Value::Object(fields) => {
                    for (key, value) in fields {
                        if matches!(
                            key.as_str(),
                            "id" | "taskId" | "slotId" | "responseGroupId" | "assetId" | "optionId"
                        ) {
                            if let Some(id) = value.as_str() {
                                ids.insert(id.into());
                            }
                        }
                        existing_ids(value, ids);
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        existing_ids(value, ids);
                    }
                }
                _ => {}
            }
        }
        let mut existing = BTreeSet::new();
        existing_ids(&current, &mut existing);
        // Legacy contexts deliberately retain whole-document edit scope.
        if context["contextMode"] == "packets" {
            for command in &commands {
                for pointer in [
                    "/taskId",
                    "/taskGroup/taskId",
                    "/replacesTaskId",
                    "/slotId",
                    "/nodeId",
                    "/parentId",
                ] {
                    if let Some(target) = command.pointer(pointer).and_then(Value::as_str) {
                        if !allowed.contains(target) {
                            return reject(format!(
                                "CLOUD_BATCH_CORRECTION_OUTSIDE_PACKET:{target}"
                            ));
                        }
                    }
                }
            }
            let footprint = crate::library::repository::EditFootprint::merge(&current, &commands);
            if let Some(target) = footprint
                .targets
                .difference(&allowed)
                .find(|target| existing.contains(*target))
            {
                return reject(format!("CLOUD_BATCH_CORRECTION_OUTSIDE_PACKET:{target}"));
            }
        }
    }
    let mut all_evidence: Vec<Value> = rulings
        .iter()
        .flat_map(|d| d["evidence"].as_array().into_iter().flatten().cloned())
        .collect();
    all_evidence.extend(evidence.iter().cloned());
    let source = evidence_source_text(request, context, packet.as_deref());
    let mut problems = tools::validate_evidence(&all_evidence);
    problems.extend(tools::verify_evidence_quotes(&all_evidence, &source).0);
    if !problems.is_empty() {
        return (
            CloudRepairToolResultV1::rejected(&call.call_id, problems),
            None,
        );
    }
    let version = context
        .pointer("/draftSlice/editVersion")
        .or_else(|| context.get("editVersion"))
        .and_then(Value::as_i64);
    let Some(base) = version else {
        return reject("CLOUD_EDIT_BASE_VERSION_MISSING".into());
    };
    if let Err(error) = decision::validate_unit_choices(request, &rulings) {
        return reject(error);
    }
    let mut result = CloudRepairToolResultV1::ok(&call.call_id, json!({"recorded":[],"errors":[]}));
    let mut applied = 0;
    if !rulings.is_empty() {
        let nested = CloudRepairToolCallV1 {
            call_id: format!("{}:choices", call.call_id),
            tool: "record_ruling".into(),
            arguments: json!({"rulings":rulings,"baseVersion":base}),
        };
        let (outcome, count) =
            execute_tool(request, &nested, round, context, packet.as_deref_mut());
        if outcome.status != crate::schema::cloud_repair_v1::CloudRepairToolStatusV1::Ok
            || !outcome.errors.is_empty()
            || outcome.result["errors"]
                .as_array()
                .is_some_and(|e| !e.is_empty())
        {
            return (outcome, count);
        }
        applied += count.unwrap_or(0);
        result = outcome;
    }
    if !commands.is_empty() {
        // Our own preceding candidate replacement is one transaction. Never read and inject
        // an arbitrary new version: a concurrent human save must still fail the CAS.
        let edit_base = base + i64::from(applied > 0);
        let nested = CloudRepairToolCallV1 {
            call_id: format!("{}:corrections", call.call_id),
            tool: "apply_edits".into(),
            arguments: json!({"baseVersion":edit_base,"commands":commands,"evidence":evidence}),
        };
        let (outcome, count) =
            execute_tool(request, &nested, round, context, packet.as_deref_mut());
        if outcome.status != crate::schema::cloud_repair_v1::CloudRepairToolStatusV1::Ok {
            // A partial candidate choice stays journalled and is visible; never claim all done.
            result.status = outcome.status;
            result.errors = outcome.errors;
            result.result["correctionErrors"] = outcome.result;
            return (result, if applied > 0 { Some(applied) } else { None });
        }
        applied += count.unwrap_or(0);
        result.result["corrections"] = outcome.result;
    }
    result.call_id = call.call_id.clone();
    result.result["compactBatch"] = json!(true);
    (result, if applied > 0 { Some(applied) } else { None })
}

fn source_line_evidence(context: &Value, id: &str) -> CommandResult<Value> {
    fn visit(value: &Value, id: &str, page: Option<u64>, matches: &mut Vec<(u64, String)>) {
        match value {
            Value::Object(fields) => {
                let page = fields.get("pageIndex").and_then(Value::as_u64).or(page);
                if let (Some(page), Some(lines)) =
                    (page, fields.get("lines").and_then(Value::as_array))
                {
                    for line in lines {
                        if line
                            .get("id")
                            .or_else(|| line.get("lineId"))
                            .and_then(Value::as_str)
                            == Some(id)
                        {
                            if let Some(text) =
                                line["text"].as_str().filter(|text| !text.trim().is_empty())
                            {
                                matches.push((page, text.into()));
                            }
                        }
                    }
                }
                for value in fields.values() {
                    visit(value, id, page, matches);
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, id, page, matches);
                }
            }
            _ => {}
        }
    }
    let mut matches = Vec::new();
    visit(&context["sourceEvidence"], id, None, &mut matches);
    matches.sort();
    matches.dedup();
    if matches.len() != 1 {
        return Err(format!(
            "CLOUD_BATCH_EVIDENCE_LINE_NOT_PROVIDED_OR_AMBIGUOUS:{id}"
        ));
    }
    let source = context
        .pointer("/sourceEvidence/sourceFileId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("CLOUD_BATCH_EVIDENCE_SOURCE_MISSING")?;
    Ok(json!({"sourceFileId":source,"pageIndex":matches[0].0,"quote":matches[0].1}))
}
