//! editor_journal_v1.change_json 的 JSON 路径级最小差异（D2）。
//!
//! 只记录改动叶子路径的改前/改后值，而不是整对象快照：拖一个选项从记整题组（约 400KB）
//! 降到几十字节。撤销时按差异反向回填。数组只在长度相同时逐元素递归，长度变化的数组作为
//! 整体记在其路径上——避免下标错位导致的错误对齐。

use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// 一条路径级差异。`path` 的每段是对象键（String）或数组下标（Number）。
///
/// `before`/`after` 用 `Option` 区分「该路径缺失」（None）与「存在且为 JSON null」（Some(Null)）：
/// 缺失表示这一侧没有该路径（新增/删除），撤销据此增删。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffEntry {
    pub path: Vec<Value>,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// 序列化为 JSON。`before`/`after` 仅在存在时写入键——serde 的 Option 会把 null 与缺失
/// 混为一谈，所以手工按键的存在与否落盘，保住「存在且为 null」这一情形。
pub(crate) fn entry_to_json(entry: &DiffEntry) -> Value {
    let mut object = Map::new();
    object.insert("path".to_string(), Value::Array(entry.path.clone()));
    if let Some(before) = &entry.before {
        object.insert("before".to_string(), before.clone());
    }
    if let Some(after) = &entry.after {
        object.insert("after".to_string(), after.clone());
    }
    Value::Object(object)
}

/// 从 JSON 还原一条差异：键存在（哪怕值是 null）→ Some；键缺失 → None。
pub(crate) fn entry_from_json(value: &Value) -> Option<DiffEntry> {
    let object = value.as_object()?;
    let path = object.get("path")?.as_array()?.clone();
    Some(DiffEntry {
        path,
        before: object.get("before").cloned(),
        after: object.get("after").cloned(),
    })
}

/// 计算 `before` → `after` 的路径级差异（两个整目标值之间）；相等返回空。
pub(crate) fn diff_values(before: &Value, after: &Value) -> Vec<DiffEntry> {
    let mut out = Vec::new();
    let mut path = Vec::new();
    diff_rec(Some(before), Some(after), &mut path, &mut out);
    out
}

fn diff_rec(before: Option<&Value>, after: Option<&Value>, path: &mut Vec<Value>, out: &mut Vec<DiffEntry>) {
    match (before, after) {
        (Some(b), Some(a)) if b == a => {}
        (Some(Value::Object(before_map)), Some(Value::Object(after_map))) => {
            let mut keys: BTreeSet<&String> = BTreeSet::new();
            keys.extend(before_map.keys());
            keys.extend(after_map.keys());
            for key in keys {
                path.push(Value::String(key.clone()));
                diff_rec(before_map.get(key), after_map.get(key), path, out);
                path.pop();
            }
        }
        (Some(Value::Array(before_items)), Some(Value::Array(after_items)))
            if before_items.len() == after_items.len() =>
        {
            for index in 0..before_items.len() {
                path.push(Value::Number(index.into()));
                diff_rec(Some(&before_items[index]), Some(&after_items[index]), path, out);
                path.pop();
            }
        }
        _ => out.push(DiffEntry {
            path: path.clone(),
            before: before.cloned(),
            after: after.cloned(),
        }),
    }
}

/// 读取 `root` 在 `path` 处的值；任一段缺失/类型不符则 None。
pub(crate) fn value_at<'a>(root: &'a Value, path: &[Value]) -> Option<&'a Value> {
    let mut current = root;
    for segment in path {
        current = match segment {
            Value::String(key) => current.as_object()?.get(key)?,
            Value::Number(number) => current.as_array()?.get(number.as_u64()? as usize)?,
            _ => return None,
        };
    }
    Some(current)
}

/// 把 `root` 在 `path` 处设为 `value`（None = 该路径应缺失：对象键删除、数组元素置 null、
/// 根整体置 null）。父路径必须已存在，否则返回 false（不凭空造中间层）。
pub(crate) fn set_at(root: &mut Value, path: &[Value], value: Option<&Value>) -> bool {
    let Some((last, parents)) = path.split_last() else {
        *root = value.cloned().unwrap_or(Value::Null);
        return true;
    };
    let mut current = root;
    for segment in parents {
        current = match segment {
            Value::String(key) => match current.as_object_mut().and_then(|object| object.get_mut(key)) {
                Some(child) => child,
                None => return false,
            },
            Value::Number(number) => {
                let Some(index) = number.as_u64() else { return false };
                match current.as_array_mut().and_then(|array| array.get_mut(index as usize)) {
                    Some(child) => child,
                    None => return false,
                }
            }
            _ => return false,
        };
    }
    match last {
        Value::String(key) => {
            let Some(object) = current.as_object_mut() else { return false };
            match value {
                Some(replacement) => { object.insert(key.clone(), replacement.clone()); }
                None => { object.remove(key); }
            }
            true
        }
        Value::Number(number) => {
            let Some(index) = number.as_u64().map(|value| value as usize) else { return false };
            let Some(array) = current.as_array_mut() else { return false };
            if index >= array.len() {
                return false;
            }
            array[index] = value.cloned().unwrap_or(Value::Null);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 反向回填：从 after 出发把每条差异的 before 写回（浅路径先），复现 before。
    fn apply_before(current: &Value, entries: &[DiffEntry]) -> Value {
        let mut out = current.clone();
        let mut ordered: Vec<&DiffEntry> = entries.iter().collect();
        ordered.sort_by_key(|entry| entry.path.len());
        for entry in ordered {
            assert!(set_at(&mut out, &entry.path, entry.before.as_ref()));
        }
        out
    }

    #[test]
    fn diff_of_one_leaf_is_minimal_and_round_trips() {
        let before = json!({ "values": ["a"], "note": "keep", "big": "xxxxxxxxxx" });
        let after = json!({ "values": ["b"], "note": "keep", "big": "xxxxxxxxxx" });
        let entries = diff_values(&before, &after);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, vec![json!("values"), json!(0)]);
        assert_eq!(apply_before(&after, &entries), before);
    }

    #[test]
    fn added_key_is_removed_on_undo() {
        let before = json!({ "a": 1 });
        let after = json!({ "a": 1, "b": 2 });
        let entries = diff_values(&before, &after);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].before, None);
        assert_eq!(entries[0].after, Some(json!(2)));
        assert_eq!(apply_before(&after, &entries), before);
    }

    #[test]
    fn removed_key_is_restored_on_undo() {
        let before = json!({ "a": 1, "b": 2 });
        let after = json!({ "a": 1 });
        assert_eq!(apply_before(&after, &diff_values(&before, &after)), before);
    }

    #[test]
    fn length_changing_array_is_recorded_whole_and_round_trips() {
        let before = json!({ "xs": [1, 2, 3] });
        let after = json!({ "xs": [1, 2] });
        let entries = diff_values(&before, &after);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, vec![json!("xs")]);
        assert_eq!(apply_before(&after, &entries), before);
    }

    #[test]
    fn present_null_survives_serialization() {
        let entry = DiffEntry { path: vec![json!("x")], before: Some(Value::Null), after: None };
        let restored = entry_from_json(&entry_to_json(&entry)).unwrap();
        assert_eq!(restored.before, Some(Value::Null));
        assert_eq!(restored.after, None);
    }
}


