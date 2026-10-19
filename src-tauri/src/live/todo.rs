//! 4.0 W1 (todo widget): props and cleaning. Drawing is in render/todo.rs. The user types the lines and ticks them in the
//! editor's Properties (the screen has no touch); the engine only draws what the props say.
//! Rules: lenient props (`#[serde(default, rename_all = "camelCase")]`), defaults behave sensibly and nothing new appears in
//! existing themes; `clean` bounds every prop (idempotent) using the helpers of `super::morewidgets`.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::morewidgets::{boolean, colour, short};
use super::theme::Widget;

pub const MAX_ITEMS: usize = 8;
pub const MAX_TITLE: usize = 30;
pub const MAX_TEXT: usize = 60;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TodoProps {
    /// None = no title.
    pub title: Option<String>,
    /// 0..8 rows. A missing list in stored JSON is an empty list (the editor creates three example rows for a NEW item).
    pub items: Vec<TodoItem>,
    /// Done rows are not drawn at all (otherwise greyed and struck through).
    pub hide_done: bool,
    /// Text colour.
    pub color: String,
    /// Tick box colour.
    pub accent: String,
}
impl Default for TodoProps {
    fn default() -> Self {
        TodoProps { title: None, items: Vec::new(), hide_done: false, color: "#ffffff".into(), accent: "#5ac8ff".into() }
    }
}

impl Widget {
    pub fn todo(&self) -> TodoProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`). Idempotent.
pub fn clean(o: &mut Map<String, Value>) {
    short(o, "title", MAX_TITLE, true);
    let items: Vec<Value> = o
        .get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_object)
                .take(MAX_ITEMS)
                .map(|r| {
                    let mut r = r.clone();
                    short(&mut r, "text", MAX_TEXT, false);
                    boolean(&mut r, "done", false);
                    r.retain(|k, _| k == "text" || k == "done");
                    Value::Object(r)
                })
                .collect()
        })
        .unwrap_or_default();
    o.insert("items".into(), json!(items));
    boolean(o, "hideDone", false);
    colour(o, "color", "#ffffff", false);
    colour(o, "accent", "#5ac8ff", false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(v: Value) -> Value {
        let mut p = v;
        crate::live::morewidgets::clean("todo", &mut p);
        p
    }

    #[test]
    fn defaults_and_missing_items_are_empty() {
        let p = cleaned(json!({}));
        assert_eq!(p["items"], json!([]));
        assert_eq!(p["title"], Value::Null);
        assert_eq!(p["hideDone"], json!(false));
        assert_eq!(p["color"], json!("#ffffff"));
        assert_eq!(p["accent"], json!("#5ac8ff"));
        let t: TodoProps = serde_json::from_value(p).unwrap();
        assert_eq!(t, TodoProps::default());
        // not an object at all
        let p = cleaned(json!("garbage"));
        assert_eq!(p["items"], json!([]));
    }

    #[test]
    fn explicit_empty_list_stays_empty() {
        let p = cleaned(json!({ "items": [] }));
        assert_eq!(p["items"], json!([]));
        assert_eq!(cleaned(p.clone()), p);
    }

    #[test]
    fn caps_and_bad_types() {
        let many: Vec<Value> = (0..12).map(|i| json!({ "text": format!("t{i}"), "done": i % 2 == 0 })).collect();
        let p = cleaned(json!({ "items": many, "title": "x".repeat(50) }));
        assert_eq!(p["items"].as_array().unwrap().len(), 8);
        assert_eq!(p["items"][0]["done"], json!(true));
        assert_eq!(p["items"][7]["text"], json!("t7"));
        assert_eq!(p["title"].as_str().unwrap().chars().count(), 30);
        let p = cleaned(json!({ "items": [{ "text": "é".repeat(100), "done": "yes" }, 5, "str", null, { "text": 7 }, { "text": "ok", "done": true, "extra": 1 }] }));
        let a = p["items"].as_array().unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(a[0]["text"].as_str().unwrap().chars().count(), 60);
        assert_eq!(a[0]["done"], json!(false));
        assert_eq!(a[1], json!({ "text": "", "done": false }));
        assert_eq!(a[2], json!({ "text": "ok", "done": true }));
        // wrong types for the other props
        let p = cleaned(json!({ "items": "nope", "title": 4, "hideDone": "x", "color": "red", "accent": 3 }));
        assert_eq!(p["items"], json!([]));
        assert_eq!(p["title"], Value::Null);
        assert_eq!(p["hideDone"], json!(false));
        assert_eq!(p["color"], json!("#ffffff"));
        assert_eq!(p["accent"], json!("#5ac8ff"));
    }

    #[test]
    fn clean_is_idempotent_and_lenient_parse() {
        let once = cleaned(json!({ "title": "Today", "hideDone": true, "color": "#AABBCC", "items": [{ "text": "a", "done": true }, { "text": "b" }] }));
        assert_eq!(once["color"], json!("#aabbcc"));
        assert_eq!(cleaned(once.clone()), once);
        let t: TodoProps = serde_json::from_value(once).unwrap();
        assert_eq!(t.title.as_deref(), Some("Today"));
        assert!(t.hide_done);
        assert_eq!(t.items.len(), 2);
        assert!(t.items[0].done && !t.items[1].done);
        // a stored widget with junk props still gives defaults
        let w = Widget { kind: "todo".into(), props: json!([1, 2]), ..Widget::default() };
        assert_eq!(w.todo(), TodoProps::default());
    }
}
