//! Unit tests of `static/time.js`, the script that shows the portal's times in the viewer's time
//! zone. They run the script under Node.js, in a fixed zone and locale with a stand-in for the
//! few DOM calls it makes, and need no browser. Without `node` on the PATH the test says it was
//! skipped and passes; the pages' own markup is checked by the other tests either way.

use std::io::Write;
use std::process::{Command, Stdio};

const CHECKS: &str = r#"
"use strict";
const assert = require("assert");
const fs = require("fs");
const vm = require("vm");
const source = fs.readFileSync(process.env.TIME_JS, "utf8");

// The pure parts, with no document.
const bare = {};
vm.runInNewContext(source, { window: bare, Intl, Date, JSON, Object, String, isNaN });
const T = bare.MagTime;
const at = new Date("2026-11-28T05:25:47Z");
const ny = "America/New_York";
const thisYear = new Date("2026-06-01T00:00:00Z");
assert.strictEqual(T.local(at, ny, "en-US", thisYear), "Nov 28, 12:25 AM EST");
assert.strictEqual(T.local(at, ny, "en-US", new Date("2025-06-01T00:00:00Z")), "Nov 28, 2026, 12:25 AM EST");
assert.strictEqual(T.local(at, "Europe/Berlin", "en-GB", thisYear), "28 Nov, 06:25 CET");
assert.strictEqual(T.local(at, "UTC", "en-US", thisYear), "Nov 28, 5:25 AM UTC");
assert.strictEqual(T.utc(at), "2026-11-28 05:25:47 UTC");
const full = T.full(at, ny, "en-US");
assert.ok(full.startsWith("Saturday, November 28, 2026 at 12:25:47"), full);
assert.ok(full.endsWith(" EST (2026-11-28 05:25:47 UTC)"), full);
assert.strictEqual(T.zoneName(new Date("2026-07-01T12:00:00Z"), ny, "en-US"), "EDT");
// Instants only; a day or anything else is not one.
assert.strictEqual(T.parse("2026-11-28T05:25:47Z").getTime(), at.getTime());
assert.strictEqual(T.parse("2026-11-28T05:25:47.123456789Z").getTime(), at.getTime() + 123);
assert.strictEqual(T.parse("2026-11-28T00:25:47-05:00").getTime(), at.getTime());
for (const bad of ["2026-11-28", "2026-11-28 05:25 UTC", "soon", "", null]) {
  assert.strictEqual(T.parse(bad), null, String(bad));
}

// The page: a stand-in document with a few <time> elements and a form, in this zone (TZ).
function element(tag, attrs, text) {
  const e = {
    nodeType: 1, tagName: tag, attrs: Object.assign({}, attrs), textContent: text || "", title: "",
    getAttribute(n) { return Object.prototype.hasOwnProperty.call(this.attrs, n) ? this.attrs[n] : null; },
    hasAttribute(n) { return Object.prototype.hasOwnProperty.call(this.attrs, n); },
    matches(sel) { return sel === "time.local-time" && tag === "time" && /\blocal-time\b/.test(this.attrs.class || ""); },
    querySelectorAll(sel) { return sel === "time.local-time" ? [] : []; },
  };
  return e;
}
const shown = element("time", { class: "local-time", datetime: "2026-11-28T05:25:47Z" }, "2026-11-28 05:25 UTC");
const relative = element("time", { class: "local-time", datetime: "2026-11-28T05:25:47Z", "data-relative": "" }, "5 min ago");
const day = element("time", { datetime: "2026-11-28" }, "2026-11-28");
const odd = element("time", { class: "local-time", datetime: "whenever" }, "whenever");
const listeners = {};
const form = { addEventListener(type, f) { listeners[type] = f; } };
const restart = element("input", { type: "datetime-local", id: "restart-at", "data-utc": "", "data-tz-field": "restart-tz" });
restart.id = "restart-at";
restart.value = "2026-11-28T05:25";
restart.form = form;
const tz = { value: "" };
const zoneLabel = element("span", { "data-zone-for": "restart-at" }, "UTC");
let observer = null;
const doc = {
  readyState: "complete",
  body: {},
  querySelectorAll(sel) {
    if (sel === "time.local-time") return [shown, relative, odd];
    if (sel === 'input[type="datetime-local"][data-tz-field]') return [restart];
    return [];
  },
  querySelector(sel) { return sel === '[data-zone-for="restart-at"]' ? zoneLabel : null; },
  getElementById(id) { return id === "restart-tz" ? tz : null; },
};
restart.ownerDocument = doc;
function Observer(callback) { observer = callback; }
Observer.prototype.observe = function () {};
const win = { document: doc, MutationObserver: Observer };
doc.nodeType = 9;
vm.runInNewContext(source, { window: win, Intl, Date, JSON, Object, String, isNaN });
const W = win.MagTime;

assert.strictEqual(shown.textContent, W.local(at));
assert.ok(/EST/.test(shown.textContent), shown.textContent);
assert.strictEqual(shown.title, W.full(at));
assert.ok(shown.title.endsWith("(2026-11-28 05:25:47 UTC)"), shown.title);
// Relative text stays, with the absolute time in the tooltip.
assert.strictEqual(relative.textContent, "5 min ago");
assert.strictEqual(relative.title, W.full(at));
// A day and an unreadable time are left as the server wrote them.
assert.strictEqual(day.textContent, "2026-11-28");
assert.strictEqual(odd.textContent, "whenever");
assert.strictEqual(odd.title, "");

// The form: UTC from the server shown in local time, the zone named, the offset sent.
assert.strictEqual(restart.value, "2026-11-28T00:25");
assert.strictEqual(zoneLabel.textContent, "your time, EST");
listeners.submit();
assert.strictEqual(tz.value, "300");
restart.value = "2026-07-01T09:00";
listeners.submit();
assert.strictEqual(tz.value, "240", "the offset of the typed day, not today's");
restart.value = "";
listeners.submit();
assert.strictEqual(tz.value, "");

// Times added later (the status refresh) are shown as they arrive.
const later = element("time", { class: "local-time", datetime: "2026-07-01T16:00:00Z" }, "2026-07-01 16:00 UTC");
const block = element("div", {});
block.querySelectorAll = (sel) => (sel === "time.local-time" ? [later] : []);
observer([{ addedNodes: [{ nodeType: 3 }, block] }]);
assert.strictEqual(later.textContent, W.local(new Date("2026-07-01T16:00:00Z")));
assert.ok(/EDT/.test(later.textContent), later.textContent);
console.log("time.js: ok");
"#;

#[test]
fn time_script_formats_in_the_viewers_zone() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static/time.js");
    let child = Command::new("node")
        .arg("-")
        .env("TZ", "America/New_York")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .env("TIME_JS", &script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("SKIPPED time_script_formats_in_the_viewers_zone: no `node` on the PATH");
            return;
        }
        Err(e) => panic!("could not start node: {e}"),
    };
    child
        .stdin
        .take()
        .unwrap()
        .write_all(CHECKS.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("time.js: ok"));
}
