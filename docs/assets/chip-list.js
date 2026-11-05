/* Chip list page (docs/coverage/chips.md) — client-side filter.
 *
 * Pure DOM filtering over the already-rendered table rows: no network, no
 * build step, works with mkdocs-material's instant navigation (the script
 * re-initialises on every page load, and navigation.instant swaps the body).
 * The row data lives in data-* attributes written by
 * scripts/generate_chip_catalog_page.py.
 */
(function () {
  "use strict";

  function init() {
    var root = document.querySelector(".lw-chips");
    if (!root || root.dataset.lwReady === "1") return;
    root.dataset.lwReady = "1";

    var q = root.querySelector("#lw-chip-q");
    var family = root.querySelector("#lw-chip-family");
    var vendor = root.querySelector("#lw-chip-vendor");
    var coverage = root.querySelector("#lw-chip-coverage");
    var count = root.querySelector("#lw-chip-count");
    var empty = root.querySelector("#lw-chip-empty");
    var rows = Array.prototype.slice.call(root.querySelectorAll("tr.lw-chip"));
    if (!rows.length) return;

    function apply() {
      var term = (q && q.value ? q.value : "").trim().toLowerCase();
      var fam = family ? family.value : "";
      var ven = vendor ? vendor.value : "";
      var cov = coverage ? coverage.value : "";
      var shown = 0;

      rows.forEach(function (row) {
        var ok = true;
        if (term && row.dataset.search.indexOf(term) === -1) ok = false;
        if (ok && fam && row.dataset.family !== fam) ok = false;
        if (ok && ven && row.dataset.vendor !== ven) ok = false;
        if (ok && cov) {
          if (cov === "excluded") {
            if (row.dataset.status !== "excluded") ok = false;
          } else if (row.dataset.coverage !== cov) {
            ok = false;
          }
        }
        row.hidden = !ok;
        if (ok) shown += 1;
      });

      if (count) {
        count.textContent =
          "Showing " + shown + " of " + rows.length + " chips.";
      }
      if (empty) empty.hidden = shown !== 0;
    }

    [q, family, vendor, coverage].forEach(function (el) {
      if (!el) return;
      el.addEventListener("input", apply);
      el.addEventListener("change", apply);
    });

    apply();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
  // mkdocs-material instant navigation: re-run after each in-page swap.
  if (typeof document$ !== "undefined" && document$.subscribe) {
    document$.subscribe(init);
  }
})();
