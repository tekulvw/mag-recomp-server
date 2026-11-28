// The "Report a player" form's helpers (docs/portal-player-reports.md). It suggests player names
// as the reporter types, from /reports/players (names only). Suggestions are set with .value only:
// nothing from the answer is ever parsed as HTML. Without this script the form's search button
// lists matching names. The shared time.js tells the server the incident time's time zone.
(function () {
  "use strict";
  var form = document.getElementById("report-form");
  if (!form) return;
  var input = document.getElementById("report-name");
  var list = document.getElementById("report-names");
  var url = form.getAttribute("data-players") || "/reports/players";
  var timer = null;
  var asked = "";

  function show(names) {
    while (list.firstChild) list.removeChild(list.firstChild);
    for (var i = 0; i < names.length && i < 10; i++) {
      if (typeof names[i] !== "string") continue;
      var option = document.createElement("option");
      option.value = names[i];
      list.appendChild(option);
    }
  }

  function lookUp() {
    var q = input.value.trim();
    if (q.length < 2 || q === asked) return;
    asked = q;
    fetch(url + "?q=" + encodeURIComponent(q), {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    })
      .then(function (r) {
        return r.ok ? r.json() : null;
      })
      .then(function (body) {
        if (body && Array.isArray(body.players) && input.value.trim() === q) show(body.players);
      })
      .catch(function () {});
  }

  if (input && list && window.fetch) {
    input.addEventListener("input", function () {
      if (timer) clearTimeout(timer);
      timer = setTimeout(lookUp, 250);
    });
  }
})();
