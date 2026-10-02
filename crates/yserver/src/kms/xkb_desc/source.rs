//! The xkeyboard-config sources on a context's include path, read where
//! xkbcomp's compile of them keeps what xkbcommon's dump no longer carries:
//! key type definitions as written ([`type_definitions`]), and the action
//! defaults xkbcomp's compat includes inherit ([`compat_actions`]).
//!
//! The context is the one the keymap was compiled in, so these are the
//! sources it was compiled from. Every reader is only a hint: the seed
//! checks what it reads against the dump before using it.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc};

use xkbcommon::xkb;

use super::text;

/// The source with its `//` and `#` comments blanked (outside strings).
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_string = false;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '#' || (c == '/' && chars.peek() == Some(&'/')) {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else {
            in_string = c == '"';
            out.push(c);
        }
    }
    out
}

/// One `[flags] xkb_<keyword> "name" { … };` of a source file.
struct SourceMap {
    default: bool,
    name: Option<String>,
    body: Vec<String>,
}

/// The maps of `src` (comments stripped) whose section keyword is one of
/// `keywords`.
fn maps(src: &str, keywords: &[&str]) -> Vec<SourceMap> {
    let mut out = Vec::new();
    for stmt in text::split_top_level(src, b';') {
        let head = stmt.split('{').next().unwrap_or_default();
        let words: Vec<&str> = head.split_whitespace().collect();
        let Some(kw) = words.iter().position(|w| {
            let w = w.split('"').next().unwrap_or_default();
            keywords.iter().any(|k| w.eq_ignore_ascii_case(k))
        }) else {
            continue;
        };
        let Ok(body) = text::body_statements(stmt) else {
            continue;
        };
        out.push(SourceMap {
            default: words[..kw]
                .iter()
                .any(|w| w.eq_ignore_ascii_case("default")),
            name: text::quoted(head),
            body: body.into_iter().map(str::to_owned).collect(),
        });
    }
    out
}

/// Every definition of every key type in the `types/` files on `ctx`'s
/// include path: name → each definition's body statements.
pub(crate) fn type_definitions(ctx: &xkb::Context) -> HashMap<String, Vec<Vec<String>>> {
    let mut defs: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for dir in ctx.include_paths() {
        let Ok(files) = std::fs::read_dir(dir.join("types")) else {
            continue;
        };
        let mut files: Vec<_> = files.filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        for file in files {
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            for map in maps(&strip_comments(&src), &["xkb_types"]) {
                for stmt in &map.body {
                    let head = stmt.split('"').next().unwrap_or_default();
                    if !head
                        .split_whitespace()
                        .last()
                        .is_some_and(|w| w.eq_ignore_ascii_case("type"))
                    {
                        continue;
                    }
                    if let (Some(name), Ok(body)) =
                        (text::quoted(stmt), text::body_statements(stmt))
                    {
                        defs.entry(name)
                            .or_default()
                            .push(body.into_iter().map(str::to_owned).collect());
                    }
                }
            }
        }
    }
    defs
}

/// xkbcomp's merge modes (`Merge*`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Merge {
    Default,
    Augment,
    Override,
    Replace,
}

fn merge_keyword(word: &str) -> Option<Merge> {
    match word.to_ascii_lowercase().as_str() {
        "include" => Some(Merge::Default),
        "augment" => Some(Merge::Augment),
        "override" => Some(Merge::Override),
        "replace" => Some(Merge::Replace),
        _ => None,
    }
}

/// An action default (`setMods.clearLocks= True;`): element, field, value.
type ActionDefault = (String, String, String);

/// An interpret's action as its source writes it, with the action defaults
/// in effect where it is defined: xkbcomp's, and xkbcommon's (each include
/// starts with none in libxkbcommon 1.13; 1.6 keeps one set for the whole
/// compile).
struct InterpAction {
    text: String,
    xkbcomp: Vec<ActionDefault>,
    xkbcommon: [Vec<ActionDefault>; 2],
}

/// xkbcomp's `SymInterpInfo`, as far as the action goes.
struct Interp<K> {
    key: K,
    merge: Merge,
    action: Option<InterpAction>,
}

/// xkbcomp's `CompatInfo` for one map being compiled.
struct Scope<K> {
    interps: Vec<Interp<K>>,
    /// `info->act`: the defaults list, shared with the includes made while
    /// it is non-empty (`HandleIncludeCompatMap` copies the head pointer
    /// and `SetActionField` appends at the tail).
    act: Option<Rc<RefCell<Vec<ActionDefault>>>>,
    /// The defaults set in this map itself.
    local: Vec<ActionDefault>,
}

impl<K> Scope<K> {
    fn new(act: Option<Rc<RefCell<Vec<ActionDefault>>>>) -> Self {
        Self {
            interps: Vec::new(),
            act,
            local: Vec::new(),
        }
    }
}

/// xkbcomp's `AddInterp`, for the action.
fn add_interp<K: PartialEq>(into: &mut Vec<Interp<K>>, new: Interp<K>) {
    let Some(old) = into.iter_mut().find(|o| o.key == new.key) else {
        into.push(new);
        return;
    };
    if new.merge == Merge::Replace {
        *old = new;
    } else if new.action.is_some() && (old.action.is_none() || new.merge != Merge::Augment) {
        old.action = new.action;
    }
}

/// xkbcomp's `MergeIncludedCompatMaps`, for the interprets.
fn merge_included<K: PartialEq>(into: &mut Scope<K>, from: Scope<K>, merge: Merge) {
    for mut si in from.interps {
        if merge != Merge::Default {
            si.merge = merge;
        }
        add_interp(&mut into.interps, si);
    }
}

/// xkbcomp's compat compile, walked for the interprets' actions.
struct CompatWalk<'a, K> {
    dirs: Vec<PathBuf>,
    files: HashMap<String, Option<Vec<SourceMap>>>,
    key_of: &'a dyn Fn(&str) -> Option<K>,
    /// Every default in compile order (libxkbcommon 1.6's view).
    global: Vec<ActionDefault>,
    failed: bool,
}

/// `include "a(m)+b|c"` → `(merge, file, map)` per piece; the first takes
/// the statement's merge, `+` overrides, `|` augments (`XkbParseIncludeMap`).
/// A `:N` suffix is dropped.
fn include_pieces(merge: Merge, s: &str) -> Vec<(Merge, String, Option<String>)> {
    let mut out = Vec::new();
    let mut op = merge;
    let mut cur = String::new();
    let mut push = |op: Merge, piece: &str| {
        let piece = piece.split(':').next().unwrap_or_default().trim();
        if piece.is_empty() {
            return;
        }
        let (file, map) = match piece.split_once('(') {
            Some((f, m)) => (f, Some(m.trim_end_matches(')').to_owned())),
            None => (piece, None),
        };
        out.push((op, file.to_owned(), map));
    };
    for c in s.chars() {
        if c == '+' || c == '|' {
            push(op, &cur);
            cur.clear();
            op = if c == '+' {
                Merge::Override
            } else {
                Merge::Augment
            };
        } else {
            cur.push(c);
        }
    }
    push(op, &cur);
    out
}

impl<K: PartialEq> CompatWalk<'_, K> {
    /// The body of `file(map)` in the first `compat/` directory that has
    /// the file: the named map, else the `default` one, else the first.
    fn find_map(&mut self, file: &str, map: Option<&str>) -> Option<Vec<String>> {
        if !self.files.contains_key(file) {
            let maps = self.dirs.iter().find_map(|d| {
                let src = std::fs::read_to_string(d.join("compat").join(file)).ok()?;
                Some(maps(
                    &strip_comments(&src),
                    &["xkb_compatibility", "xkb_compat", "xkb_compatibility_map"],
                ))
            });
            self.files.insert(file.to_owned(), maps);
        }
        let maps = self.files.get(file)?.as_ref()?;
        let found = match map {
            Some(m) => maps.iter().find(|x| x.name.as_deref() == Some(m)),
            None => maps.iter().find(|x| x.default).or_else(|| maps.first()),
        };
        found.map(|m| m.body.clone())
    }

    fn set_default(&mut self, scope: &mut Scope<K>, d: ActionDefault) {
        match &scope.act {
            Some(list) => list.borrow_mut().push(d.clone()),
            None => scope.act = Some(Rc::new(RefCell::new(vec![d.clone()]))),
        }
        scope.local.push(d.clone());
        self.global.push(d);
    }

    /// `elem.field= value` outside `interpret.` / `indicator.`: an action
    /// default.
    fn action_default(stmt: &str) -> Option<ActionDefault> {
        if stmt.contains('{') {
            return None;
        }
        let (lhs, _, value) = text::field(stmt.trim_end_matches(';'));
        let (elem, field) = lhs.split_once('.')?;
        let elem = elem.trim().to_ascii_lowercase();
        if elem == "interpret"
            || elem == "indicator"
            || !elem.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return None;
        }
        Some((elem, field.trim().to_owned(), value?.to_owned()))
    }

    /// xkbcomp's `HandleIncludeCompatMap`.
    fn include(&mut self, merge: Merge, s: &str, scope: &mut Scope<K>, depth: usize) {
        let mut included: Option<(Merge, Scope<K>)> = None;
        for (op, file, map) in include_pieces(merge, s) {
            let Some(body) = self.find_map(&file, map.as_deref()) else {
                self.failed = true;
                return;
            };
            let mut next = Scope::new(scope.act.clone());
            self.handle_map(&body, Merge::Override, &mut next, depth + 1);
            match &mut included {
                None => included = Some((op, next)),
                Some((_, into)) => merge_included(into, next, op),
            }
        }
        if let Some((op, from)) = included {
            merge_included(scope, from, op);
        }
    }

    /// xkbcomp's `HandleCompatMapFile`, for includes, action defaults and
    /// interprets.
    fn handle_map(&mut self, body: &[String], merge: Merge, scope: &mut Scope<K>, depth: usize) {
        if depth > 15 {
            self.failed = true;
            return;
        }
        for stmt in body {
            // An include ends without a `;`: a statement may start with any
            // number of them.
            let mut stmt = stmt.as_str();
            loop {
                if self.failed {
                    return;
                }
                let first = stmt.split_whitespace().next().unwrap_or_default();
                let after = stmt[first.len()..].trim_start();
                let (Some(m), Some(end)) = (
                    merge_keyword(first),
                    after.strip_prefix('"').and_then(|a| a.find('"')),
                ) else {
                    break;
                };
                self.include(m, &after[1..=end], scope, depth);
                stmt = after[end + 2..].trim_start();
            }
            let first = stmt.split_whitespace().next().unwrap_or_default();
            let stmt_merge = merge_keyword(first);
            let rest = match stmt_merge {
                Some(_) => stmt[first.len()..].trim_start(),
                None => stmt,
            };
            if let Some(head) = rest.strip_prefix("interpret")
                && head.starts_with(char::is_whitespace)
            {
                let head = head.split('{').next().unwrap_or_default().trim();
                let Some(key) = (self.key_of)(head) else {
                    continue;
                };
                let mut action = None;
                for f in text::body_statements(rest).unwrap_or_default() {
                    if let Some(d) = Self::action_default(f) {
                        self.set_default(scope, d);
                    } else if let (k, None, Some(v)) = text::field(f)
                        && k.eq_ignore_ascii_case("action")
                    {
                        action = Some(InterpAction {
                            text: v.to_owned(),
                            xkbcomp: scope
                                .act
                                .as_ref()
                                .map(|l| l.borrow().clone())
                                .unwrap_or_default(),
                            xkbcommon: [scope.local.clone(), self.global.clone()],
                        });
                    }
                }
                let merge = match stmt_merge {
                    Some(m) if m != Merge::Default => m,
                    _ => merge,
                };
                add_interp(&mut scope.interps, Interp { key, merge, action });
            } else if let Some(d) = Self::action_default(rest) {
                self.set_default(scope, d);
            }
        }
    }
}

/// An interpret's action as xkbcomp compiles it from the compat component
/// xkbcommon named its section after (`section`, escaped:
/// `complete_grp_led(scroll)`) on `ctx`'s include path, and as xkbcommon
/// may have: key (from `key_of` over the interpret's head), xkbcomp's
/// action text, and xkbcommon's candidates. `None` when a source isn't
/// found.
///
/// xkbcomp's includes inherit the defaults of the map including them
/// (`HandleIncludeCompatMap`: `included.act = info->act`), so a default set
/// before `include "misc(assign_shift_left_action)"` reaches that map's
/// `SetMods`; xkbcommon's don't.
pub(crate) fn compat_actions<K: PartialEq>(
    ctx: &xkb::Context,
    section: &str,
    key_of: &dyn Fn(&str) -> Option<K>,
) -> Option<Vec<(K, String, [String; 2])>> {
    let dirs: Vec<PathBuf> = ctx.include_paths().map(PathBuf::from).collect();
    // The escape made `+` and `:` into `_`; an `_` stays where it joins
    // two pieces into a compat file's name (`grp_led`).
    let is_file = |piece: &str| {
        let file = piece.split('(').next().unwrap_or_default();
        dirs.iter().any(|d| d.join("compat").join(file).is_file())
    };
    let mut pieces: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in section.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '_' if depth == 0 => {
                pieces.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    pieces.push(cur);
    let mut component = String::new();
    let mut file = String::new();
    for p in pieces {
        if p.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !file.is_empty() && !is_file(&file) && is_file(&format!("{file}_{p}")) {
            file = format!("{file}_{p}");
            continue;
        }
        if !file.is_empty() {
            component.push_str(&file);
            component.push('+');
        }
        file = p;
    }
    component.push_str(&file);
    let mut walk = CompatWalk {
        dirs,
        files: HashMap::new(),
        key_of,
        global: Vec::new(),
        failed: false,
    };
    let mut root = Scope::new(None);
    walk.include(Merge::Default, &component, &mut root, 0);
    if walk.failed {
        return None;
    }
    Some(
        root.interps
            .into_iter()
            .filter_map(|si| {
                let a = si.action?;
                let [local, global] = &a.xkbcommon;
                Some((
                    si.key,
                    with_defaults(&a.text, &a.xkbcomp),
                    [
                        with_defaults(&a.text, local),
                        with_defaults(&a.text, global),
                    ],
                ))
            })
            .collect(),
    )
}

/// xkbcomp's action name (`stringToAction`) in one spelling.
fn action_name(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    match n.as_str() {
        "movepointer" => "moveptr",
        "pointerbutton" => "ptrbtn",
        "lockpointerbutton" | "lockptrbutton" | "lockpointerbtn" => "lockptrbtn",
        "setpointerdefault" => "setptrdflt",
        "terminateserver" => "terminate",
        "messageaction" | "message" => "actionmessage",
        "redirect" => "redirectkey",
        "devbtn" | "devbutton" | "devicebutton" => "devicebtn",
        "lockdevbtn" | "lockdevbutton" | "lockdevicebutton" => "lockdevicebtn",
        "devval" | "deviceval" | "devvaluator" => "devicevaluator",
        _ => return n,
    }
    .to_owned()
}

/// xkbcomp's field name (`stringToField`) in one spelling.
fn field_name(name: &str) -> String {
    let n = name.trim().to_ascii_lowercase();
    match n.as_str() {
        "mods" => "modifiers",
        "generatekeyevent" => "genkeyevent",
        "accelerate" | "repeat" => "accel",
        "ctrls" => "controls",
        "sameserver" => "same",
        "dev" => "device",
        "key" | "kc" => "keycode",
        "clearmodifiers" => "clearmods",
        _ => return n,
    }
    .to_owned()
}

/// `action` with `defaults` applied first (`HandleActionDef`: the defaults
/// for its type, or for any action, then its own arguments), in the dump's
/// spelling: a true flag bare, a false one left out.
fn with_defaults(action: &str, defaults: &[ActionDefault]) -> String {
    let (name, args) = match action.split_once('(') {
        Some((n, a)) => (n.trim(), a.trim_end().strip_suffix(')').unwrap_or(a)),
        None => (action.trim(), ""),
    };
    let kind = action_name(name);
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut set = |field: &str, value: Option<&str>| {
        let key = field_name(field);
        out.retain(|(k, _)| *k != key);
        let arg = match value.map(|v| v.trim().to_ascii_lowercase()) {
            None => Some(field.trim().to_owned()),
            Some(v) if matches!(v.as_str(), "true" | "yes" | "on") => Some(field.trim().to_owned()),
            Some(v) if matches!(v.as_str(), "false" | "no" | "off") => None,
            Some(_) => Some(format!(
                "{}={}",
                field.trim(),
                value.unwrap_or_default().trim()
            )),
        };
        out.push((key, arg));
    };
    for (elem, field, value) in defaults {
        if elem == "action" || action_name(elem) == kind {
            set(field, Some(value));
        }
    }
    for arg in text::split_top_level(args, b',') {
        match arg.split_once('=') {
            Some((f, v)) => set(f, Some(v)),
            None => match arg.strip_prefix(['!', '~']) {
                Some(f) => set(f, Some("false")),
                None => set(arg, None),
            },
        }
    }
    let args: Vec<String> = out.into_iter().filter_map(|(_, a)| a).collect();
    format!("{name}({})", args.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_defaults_puts_the_arguments_last() {
        let d = |e: &str, f: &str, v: &str| (e.to_owned(), f.to_owned(), v.to_owned());
        assert_eq!(
            with_defaults(
                "SetMods(modifiers = Shift)",
                &[
                    d("setmods", "clearLocks", "True"),
                    d("latchmods", "latchToLock", "True")
                ]
            ),
            "SetMods(clearLocks,modifiers=Shift)"
        );
        assert_eq!(
            with_defaults(
                "LatchMods(mods=Shift,!clearLocks)",
                &[d("latchmods", "clearLocks", "True")]
            ),
            "LatchMods(mods=Shift)"
        );
    }

    /// Xvfb 21.1.24 (xkbcomp 1.5.0) on xkeyboard-config 2.41 and on two
    /// edits of it: 2.41's `misc(assign_shift_left_action)` `Shift_L` gets
    /// `clearLocks` from `misc`'s own default (`act=0101…`); moved into
    /// `complete` beside `basic`, it gets none of `basic`'s (`0100…`); a
    /// default set in a map included after `misc`'s own reaches `misc`'s
    /// later `LatchMods` (`0202…`).
    #[test]
    fn compat_defaults_follow_xkbcomp_includes() {
        let root = std::env::temp_dir().join(format!("yserver-xkb-compat-{}", std::process::id()));
        let files = [
            (
                "complete",
                r#"default xkb_compatibility "complete" {
                    include "basic"
                    augment "sibling(shift)"
                    augment "misc"
                };"#,
            ),
            (
                "basic",
                r#"default xkb_compatibility "basic" {
                    setMods.clearLocks= True;
                    interpret Any+AnyOf(all) { action= SetMods(modifiers=modMapMods,clearLocks); };
                };"#,
            ),
            (
                "sibling",
                r#"xkb_compatibility "shift" {
                    interpret Shift_R { action = SetMods(modifiers = Shift); };
                };"#,
            ),
            (
                "latch",
                r#"default xkb_compatibility "latch" { latchMods.latchToLock= True; };"#,
            ),
            (
                "misc",
                r#"// comment "with a quote
                default partial xkb_compatibility "misc" {
                    setMods.clearLocks= True;
                    include "latch"
                    interpret Hyper_L { action = LatchMods(modifiers=Hyper); };
                    include "misc(assign_shift_left_action)"
                };
                partial xkb_compatibility "assign_shift_left_action" {
                    interpret Shift_L { action = SetMods(modifiers = Shift); };
                };"#,
            ),
        ];
        std::fs::create_dir_all(root.join("compat")).unwrap();
        for (name, src) in files {
            std::fs::write(root.join("compat").join(name), src).unwrap();
        }
        let mut ctx = xkb::Context::new(xkb::CONTEXT_NO_DEFAULT_INCLUDES);
        assert!(ctx.include_path_append(&root));
        let got = compat_actions(&ctx, "complete", &|h: &str| Some(h.to_owned()));
        std::fs::remove_dir_all(&root).unwrap();
        let got = got.expect("every source found");
        let find = |k: &str| got.iter().find(|g| g.0 == k).cloned().unwrap();
        assert_eq!(find("Shift_R").1, "SetMods(modifiers=Shift)");
        let shift_l = find("Shift_L");
        assert_eq!(shift_l.1, "SetMods(clearLocks,modifiers=Shift)");
        assert_eq!(shift_l.2[0], "SetMods(modifiers=Shift)");
        let hyper = find("Hyper_L");
        assert_eq!(hyper.1, "LatchMods(latchToLock,modifiers=Hyper)");
        assert_eq!(hyper.2[0], "LatchMods(modifiers=Hyper)");
        assert_eq!(
            find("Any+AnyOf(all)").1,
            "SetMods(modifiers=modMapMods,clearLocks)"
        );
    }

    #[test]
    fn include_pieces_split_as_xkbcomp() {
        let p = include_pieces(
            Merge::Default,
            "complete+ledscroll(group_lock)|japan(kana_lock):2",
        );
        let p: Vec<_> = p
            .iter()
            .map(|(_, f, m)| (f.as_str(), m.as_deref()))
            .collect();
        assert_eq!(
            p,
            [
                ("complete", None),
                ("ledscroll", Some("group_lock")),
                ("japan", Some("kana_lock"))
            ]
        );
    }
}
