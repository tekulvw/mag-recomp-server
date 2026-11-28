// Shows the portal's times in the viewer's own time zone. Pages render every instant as
// <time class="local-time" datetime="2026-11-28T05:25:47Z">2026-11-28 05:25 UTC</time>; this
// script rewrites the text in the browser's locale and zone ("Nov 28, 12:25 AM EST") and puts
// the full local time and the UTC time in its tooltip. A <time> marked data-relative keeps its
// text ("5 min ago") and only gets the tooltip, and a date-only datetime ("2026-11-28") is left
// alone, so a day never moves to a neighbouring one. Times added to the page later (the status
// block's refresh) are formatted as they arrive.
//
// Forms: a datetime-local input with data-tz-field="<id>" is typed in local time. On submit the
// hidden input <id> gets Date.getTimezoneOffset() for the typed value, which the server uses to
// store it in UTC. With data-utc the server filled the input in UTC, and the script shows that
// in local time first. An element with data-zone-for="<input id>" names the zone.
//
// Without this script every time stays in UTC, labelled, and form times are taken as UTC.
(function (global) {
  "use strict";

  var INSTANT = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/;
  var cache = {};

  // An Intl formatter, cached; null when the browser cannot make one.
  function formatter(options, zone, locale) {
    var key = JSON.stringify([options, zone || "", locale || ""]);
    if (Object.prototype.hasOwnProperty.call(cache, key)) return cache[key];
    var o = {};
    for (var k in options) o[k] = options[k];
    if (zone) o.timeZone = zone;
    var f = null;
    try {
      f = new Intl.DateTimeFormat(locale, o);
    } catch (e) {
      f = null;
    }
    cache[key] = f;
    return f;
  }

  // An instant from a datetime attribute; null for a date-only value or anything else.
  function parse(text) {
    if (typeof text !== "string" || !INSTANT.test(text)) return null;
    // Browsers agree on at most three fraction digits.
    var t = Date.parse(text.replace(/(\.\d{3})\d+/, "$1"));
    return isNaN(t) ? null : new Date(t);
  }

  function pad(n) {
    return (n < 10 ? "0" : "") + n;
  }

  // "2026-11-28 05:25:47 UTC".
  function utc(date) {
    return date.getUTCFullYear() + "-" + pad(date.getUTCMonth() + 1) + "-" + pad(date.getUTCDate()) +
      " " + pad(date.getUTCHours()) + ":" + pad(date.getUTCMinutes()) + ":" + pad(date.getUTCSeconds()) + " UTC";
  }

  function part(f, date, type) {
    if (!f || !f.formatToParts) return "";
    var parts = f.formatToParts(date);
    for (var i = 0; i < parts.length; i++) if (parts[i].type === type) return parts[i].value;
    return "";
  }

  // The short zone name at that instant, such as "EST" or "GMT+1".
  function zoneName(date, zone, locale) {
    return part(formatter({ timeZoneName: "short" }, zone, locale), date, "timeZoneName");
  }

  // "Nov 28, 12:25 AM EST", with the year when it is not this year's; UTC text without Intl.
  function local(date, zone, locale, now) {
    var year = formatter({ year: "numeric" }, zone, locale);
    var options = { month: "short", day: "numeric", hour: "numeric", minute: "2-digit", timeZoneName: "short" };
    if (!year || year.format(date) !== year.format(now || new Date())) options.year = "numeric";
    var f = formatter(options, zone, locale);
    return f ? f.format(date) : utc(date).replace(/:\d{2} UTC$/, " UTC");
  }

  // "Friday, November 28, 2026 at 12:25:47 AM EST (2026-11-28 05:25:47 UTC)".
  function full(date, zone, locale) {
    var f = formatter({ dateStyle: "full", timeStyle: "long" }, zone, locale);
    return f ? f.format(date) + " (" + utc(date) + ")" : utc(date);
  }

  // A datetime-local value, "2026-11-28T00:25", in the browser's zone.
  function inputValue(date) {
    return date.getFullYear() + "-" + pad(date.getMonth() + 1) + "-" + pad(date.getDate()) +
      "T" + pad(date.getHours()) + ":" + pad(date.getMinutes());
  }

  function show(el) {
    var date = parse(el.getAttribute("datetime"));
    if (!date) return;
    el.title = full(date);
    if (!el.hasAttribute("data-relative")) el.textContent = local(date);
  }

  // Formats every time in `root` (an element, or the whole document).
  function localize(root) {
    if (!root) return;
    if (root.nodeType === 1 && root.matches && root.matches("time.local-time")) show(root);
    if (!root.querySelectorAll) return;
    var found = root.querySelectorAll("time.local-time");
    for (var i = 0; i < found.length; i++) show(found[i]);
  }

  function setUpInput(input) {
    var doc = input.ownerDocument;
    if (input.hasAttribute("data-utc") && input.value) {
      var given = Date.parse(input.value + "Z");
      if (!isNaN(given)) input.value = inputValue(new Date(given));
    }
    var label = input.id ? doc.querySelector('[data-zone-for="' + input.id + '"]') : null;
    if (label) {
      var typed = input.value ? new Date(input.value) : new Date();
      var zone = zoneName(isNaN(typed.getTime()) ? new Date() : typed);
      label.textContent = zone ? "your time, " + zone : "your time";
    }
    var tz = doc.getElementById(input.getAttribute("data-tz-field"));
    if (tz && input.form) {
      input.form.addEventListener("submit", function () {
        var typed = new Date(input.value);
        tz.value = input.value && !isNaN(typed.getTime()) ? String(typed.getTimezoneOffset()) : "";
      });
    }
  }

  var api = { parse: parse, utc: utc, local: local, full: full, zoneName: zoneName, inputValue: inputValue, localize: localize };
  global.MagTime = api;

  var doc = global.document;
  if (!doc || !doc.querySelectorAll) return;

  function start() {
    localize(doc);
    var inputs = doc.querySelectorAll('input[type="datetime-local"][data-tz-field]');
    for (var i = 0; i < inputs.length; i++) setUpInput(inputs[i]);
    if (global.MutationObserver && doc.body) {
      new global.MutationObserver(function (records) {
        for (var r = 0; r < records.length; r++) {
          var added = records[r].addedNodes;
          for (var n = 0; n < added.length; n++) if (added[n].nodeType === 1) localize(added[n]);
        }
      }).observe(doc.body, { childList: true, subtree: true });
    }
  }

  if (doc.readyState === "loading") doc.addEventListener("DOMContentLoaded", start);
  else start();
})(typeof window !== "undefined" ? window : this);
