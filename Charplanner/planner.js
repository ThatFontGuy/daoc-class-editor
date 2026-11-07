// Divided Realms Character Planner - vanilla JS, no build step, no framework (this whole folder
// is meant to be copy-pasted straight onto a webserver, nothing to compile first).
//
// Spec-point math (totalSpecPoints/specTrainingCost below) is ported directly from the real
// server's own GamePlayer.cs (VerifySpecPoints) - not re-derived or guessed. Two real limitations
// of this port, both intentional and both called out in the UI itself rather than silently
// pretending to be exact:
//   1. Autotrain (a class's own "free points up to level/4" discount for specific specs, e.g.
//      Warrior's own Parry) isn't modeled - which specs are autotrainable for a given class is
//      hardcoded per-class in that class's own .cs source, not in the database at all, so this
//      planner (which only ever reads from the database export) has no way to know it. A spec's
//      shown cost may therefore be higher here than what training it actually costs in-game.
//   2. Realm Ability costs are shown and summed for information, but no hard point budget is
//      enforced - how many RA points a character actually has depends on Realm Rank, which this
//      planner (a hypothetical, not-yet-created character) has no meaningful value for.

(function () {
  "use strict";

  const REALM_ORDER = ["Albion", "Midgard", "Hibernia"];

  // Class.Profession is exported as the server's own raw, untranslated language key (e.g.
  // "PlayerClass.Profession.ChurchofAlbion") - the real server only ever resolves this through
  // its own in-game translation table at runtime, which this planner (a static export with no
  // server connection at all) has no access to. A small curated table reads better than showing
  // the raw key verbatim - covers every profession this server's own 52 real classes actually
  // use (confirmed directly against the generated data), with a best-effort CamelCase-splitting
  // fallback for anything this table doesn't recognize (a future class, or a different fork).
  const PROFESSION_NAMES = {
    "PlayerClass.Profession.Academy": "Academy",
    "PlayerClass.Profession.ChurchofAlbion": "Church of Albion",
    "PlayerClass.Profession.DefendersofAlbion": "Defenders of Albion",
    "PlayerClass.Profession.GuildofShadows": "Guild of Shadows",
    "PlayerClass.Profession.HouseofBodgar": "House of Bodgar",
    "PlayerClass.Profession.HouseofBragi": "House of Bragi",
    "PlayerClass.Profession.HouseofEir": "House of Eir",
    "PlayerClass.Profession.HouseofHel": "House of Hel",
    "PlayerClass.Profession.HouseofModi": "House of Modi",
    "PlayerClass.Profession.HouseofOdin": "House of Odin",
    "PlayerClass.Profession.HouseofSkadi": "House of Skadi",
    "PlayerClass.Profession.HouseofThor": "House of Thor",
    "PlayerClass.Profession.HouseofTyr": "House of Tyr",
    "PlayerClass.Profession.HouseofYmir": "House of Ymir",
    "PlayerClass.Profession.Loki": "Loki",
    "PlayerClass.Profession.PathofAffinity": "Path of Affinity",
    "PlayerClass.Profession.PathofEssence": "Path of Essence",
    "PlayerClass.Profession.PathofFocus": "Path of Focus",
    "PlayerClass.Profession.PathofHarmony": "Path of Harmony",
    "PlayerClass.Profession.TempleofArawn": "Temple of Arawn",
    "PlayerClass.Profession.TempleofIronFist": "Temple of Iron Fist",
  };
  function professionName(raw) {
    if (!raw) return "";
    if (PROFESSION_NAMES[raw]) return PROFESSION_NAMES[raw];
    return raw.replace(/^PlayerClass\.Profession\./, "").replace(/([a-z])([A-Z])/g, "$1 $2");
  }

  /** @type {{generated_at_unix:number, max_level:number, classes:any[]}} */
  let DATA = null;
  let state = {
    realm: "Albion",
    classId: null,
    level: 50,
    realmRank: 1, // RR1L0 (a brand new character) by default
    realmSubLevel: 0,
    specLevels: {}, // key_name -> trained level
    raLevels: {}, // key_name -> trained level
  };

  function $(sel, root) {
    return (root || document).querySelector(sel);
  }
  function el(tag, attrs, children) {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v == null) continue; // skip null/undefined - setAttribute would otherwise stringify
      // it to the literal text "null", which browsers treat as *present* for boolean attributes
      // like "selected"/"disabled" - a real bug found live: every <option> ended up marked
      // selected="null" (so the dropdown visually showed whichever happened to be last in DOM
      // order, not the class actually selected in state), and every stepper button in the Realm
      // Abilities table was permanently disabled="null" regardless of its real 0/max position.
      if (k === "class") node.className = v;
      else if (k === "text") node.textContent = v;
      else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
    for (const child of children || []) {
      if (child == null) continue;
      node.appendChild(typeof child === "string" ? document.createTextNode(child) : child);
    }
    return node;
  }

  // ---- Real server spec-point formula (GamePlayer.cs: VerifySpecPoints) ----
  function totalSpecPoints(level, multiplier) {
    let allpoints = -1;
    for (let i = 1; i <= level; i++) {
      if (i <= 5) allpoints += i;
      if (i > 5) allpoints += Math.floor((multiplier * i) / 10);
      if (i > 40) allpoints += Math.floor((multiplier * (i - 1)) / 20);
    }
    return Math.max(0, allpoints);
  }
  // Real server formula (GamePlayer.cs: VerifySpecPoints's own usedpoints loop):
  // (specLevel * (specLevel + 1) - 2) / 2
  function specTrainingCost(specLevel) {
    if (specLevel <= 0) return 0;
    return Math.floor((specLevel * (specLevel + 1) - 2) / 2);
  }

  // Real server formula (GamePlayer.cs: GetAutoTrainPoints, mode 0 - "sum of all AT points"):
  // max_autotrain = max(1, floor(level/4)); refund = specTrainingCost(min(max_autotrain, specLevel))
  function maxAutotrainLevel(level) {
    return Math.max(1, Math.floor(level / 4));
  }

  // Whether `spec` is one of this class's own autotrained specs - matched by the real spec
  // *KeyName*, not its display Name (confirmed directly in GamePlayer.cs's own
  // GetAutoTrainPoints: `autotrainKey == spec.KeyName`), which matters: a spec this tool actually
  // cloned for a custom class gets a `_<classId>`-suffixed KeyName that no longer matches the
  // class's own hardcoded AutotrainableSkills constant at all - the same way the real game itself
  // would never recognize a renamed/cloned key as the literal spec it was hardcoded to autotrain.
  // A mechanic-locked spec that's never actually cloned (e.g. Drengr's own "Spear"/"Staff") keeps
  // its exact original KeyName, so it still matches correctly either way.
  function isAutotrainable(cls, spec) {
    return (cls.autotrainable_specs || []).includes(spec.key_name);
  }

  // The real effective cost to train `specLevel` - specTrainingCost minus whatever the class's
  // own autotrain already refunds for free, if this spec is actually autotrainable for it.
  function effectiveSpecCost(specLevel, autotrainable, level) {
    const full = specTrainingCost(specLevel);
    if (!autotrainable) return full;
    const refund = specTrainingCost(Math.min(maxAutotrainLevel(level), specLevel));
    return full - refund;
  }

  // The real "RRxLy" notation (e.g. the user's own familiar "RR5L0") is just RealmLevel split
  // into tens/ones - confirmed directly in the server's own source (GamePlayer.cs's
  // RealmRankTitle: `RR = RealmLevel / 10 + 1`, and its own GainRealmPoints logging:
  // `RealmLevel % 10`).
  function realmLevel() {
    return (state.realmRank - 1) * 10 + state.realmSubLevel;
  }

  // Real server formula (AbstractServerRules.GetPlayerRealmPointsTotal, confirmed NOT overridden
  // by NormalServerRules - the real GameType this server's own serverconfig.xml is set to) - the
  // total Realm Ability point pool a character has, from its own Realm Rank/Level alone.
  function totalRaPoints(level, rl) {
    return level > 19 ? Math.max(1, rl) : rl;
  }

  function classesForRealm(realm) {
    return DATA.classes
      .filter((c) => c.realm === realm)
      .sort((a, b) => a.name.localeCompare(b.name));
  }

  function currentClass() {
    return DATA.classes.find((c) => c.id === state.classId) || null;
  }

  function specCap() {
    return Math.min(state.level, DATA.max_level);
  }

  // ---- URL state (shareable/bookmarkable, matching the real game planner convention) ----
  function loadFromUrl() {
    const params = new URLSearchParams(location.search);
    const c = params.get("c");
    const l = parseInt(params.get("l"), 10);
    if (c) {
      const found = DATA.classes.find((cl) => cl.name === c);
      if (found) {
        state.classId = found.id;
        state.realm = found.realm;
      }
    }
    if (!Number.isNaN(l) && l >= 1 && l <= DATA.max_level) {
      state.level = l;
    }
    const rr = params.get("rr");
    if (rr) {
      const m = /^(\d+)L(\d+)$/.exec(rr);
      if (m) {
        state.realmRank = parseInt(m[1], 10) || 1;
        state.realmSubLevel = parseInt(m[2], 10) || 0;
      }
    }
    const specs = params.get("specs");
    if (specs) {
      for (const pair of specs.split(",")) {
        const [k, v] = pair.split(":");
        if (k) state.specLevels[k] = parseInt(v, 10) || 0;
      }
    }
    const ras = params.get("ras");
    if (ras) {
      for (const pair of ras.split(",")) {
        const [k, v] = pair.split(":");
        if (k) state.raLevels[k] = parseInt(v, 10) || 0;
      }
    }
  }

  function syncUrl() {
    const cls = currentClass();
    const params = new URLSearchParams();
    if (cls) params.set("c", cls.name);
    params.set("l", String(state.level));
    params.set("rr", `${state.realmRank}L${state.realmSubLevel}`);
    const specPairs = Object.entries(state.specLevels)
      .filter(([, v]) => v > 0)
      .map(([k, v]) => `${k}:${v}`);
    if (specPairs.length) params.set("specs", specPairs.join(","));
    const raPairs = Object.entries(state.raLevels)
      .filter(([, v]) => v > 0)
      .map(([k, v]) => `${k}:${v}`);
    if (raPairs.length) params.set("ras", raPairs.join(","));
    history.replaceState(null, "", `?${params.toString()}`);
  }

  // ---- Rendering ----
  function render() {
    const app = $("#app");
    app.innerHTML = "";
    app.appendChild(renderToolbar());

    const cls = currentClass();
    if (!cls) {
      app.appendChild(el("p", { class: "hint" }, ["Pick a class above to begin planning."]));
      return;
    }

    app.appendChild(renderClassSummary(cls));
    app.appendChild(renderSpecPanel(cls));
    if (cls.realm_abilities.length) {
      app.appendChild(renderRealmAbilityPanel(cls));
    }
    syncUrl();
  }

  function renderToolbar() {
    const panel = el("div", { class: "panel" });
    const realmTabs = el(
      "div",
      { class: "realm-tabs" },
      REALM_ORDER.filter((r) => DATA.classes.some((c) => c.realm === r)).map((r) =>
        el("button", {
          class: `realm-${r}${state.realm === r ? " active" : ""}`,
          text: r,
          onclick: () => {
            state.realm = r;
            const firstInRealm = classesForRealm(r)[0];
            state.classId = firstInRealm ? firstInRealm.id : null;
            render();
          },
        })
      )
    );

    const classSelect = el(
      "select",
      {
        onchange: (e) => {
          state.classId = parseInt(e.target.value, 10);
          state.specLevels = {};
          state.raLevels = {};
          render();
        },
      },
      classesForRealm(state.realm).map((c) =>
        el("option", {
          value: String(c.id),
          selected: c.id === state.classId ? "selected" : null,
          text: `${c.name}${c.disabled ? " (disabled)" : ""}${c.is_custom ? " (custom)" : ""}`,
        })
      )
    );

    const levelInput = el("input", {
      type: "number",
      min: "1",
      max: String(DATA.max_level),
      value: String(state.level),
      style: "width: 4.5rem",
      onchange: (e) => {
        const v = Math.max(1, Math.min(DATA.max_level, parseInt(e.target.value, 10) || 1));
        state.level = v;
        render();
      },
    });

    const rrSelect = el(
      "select",
      {
        onchange: (e) => {
          state.realmRank = parseInt(e.target.value, 10) || 1;
          render();
        },
      },
      Array.from({ length: 14 }, (_, i) => i + 1).map((rr) =>
        el("option", { value: String(rr), selected: rr === state.realmRank ? "selected" : null, text: `RR${rr}` })
      )
    );
    const rlSelect = el(
      "select",
      {
        onchange: (e) => {
          state.realmSubLevel = parseInt(e.target.value, 10) || 0;
          render();
        },
      },
      Array.from({ length: 10 }, (_, i) => i).map((l) =>
        el("option", { value: String(l), selected: l === state.realmSubLevel ? "selected" : null, text: `L${l}` })
      )
    );

    const resetButton = el("button", {
      text: "Reset all training",
      onclick: () => {
        state.specLevels = {};
        state.raLevels = {};
        render();
      },
    });

    const toolbar = el("div", { class: "planner-toolbar" }, [
      el("div", { class: "field" }, [el("span", { text: "Realm" }), realmTabs]),
      el("div", { class: "field" }, [el("span", { text: "Class" }), classSelect]),
      el("div", { class: "field" }, [el("span", { text: "Level" }), levelInput]),
      el("div", { class: "field" }, [
        el("span", { text: "Realm Rank" }),
        el("div", { style: "display:flex; gap:0.3rem" }, [rrSelect, rlSelect]),
      ]),
      el("div", { class: "field", style: "margin-left:auto" }, [el("span", { text: "\u00a0" }), resetButton]),
    ]);
    panel.appendChild(toolbar);
    return panel;
  }

  function renderClassSummary(cls) {
    const panel = el("div", { class: "panel" });
    panel.appendChild(el("h2", { text: cls.name }));
    const bits = [
      professionName(cls.profession),
      [cls.primary_stat, cls.secondary_stat, cls.tertiary_stat].filter(Boolean).join(" / "),
      cls.eligible_races.length ? `Races: ${[...new Set(cls.eligible_races)].join(", ")}` : null,
    ].filter(Boolean);
    panel.appendChild(el("p", { class: "hint", text: bits.join(" \u2014 ") }));
    if (cls.disabled) {
      panel.appendChild(el("span", { class: "badge disabled", text: "Currently disabled on this server" }));
    }
    return panel;
  }

  function renderSpecPanel(cls) {
    const panel = el("div", { class: "panel" });
    panel.appendChild(el("h2", { text: "Specializations" }));

    const total = totalSpecPoints(state.level, cls.spec_points_multiplier || 0);
    let spent = 0;
    for (const spec of cls.specs) {
      spent += effectiveSpecCost(state.specLevels[spec.key_name] || 0, isAutotrainable(cls, spec), state.level);
    }
    const remaining = total - spent;
    panel.appendChild(
      el("div", { class: "points-summary" }, [
        el("span", {}, ["Total points at level ", el("strong", { text: String(state.level) }), ": ", el("strong", { text: String(total) })]),
        el("span", {}, ["Spent: ", el("strong", { text: String(spent) })]),
        el("span", {}, [
          "Remaining: ",
          el("strong", {
            class: remaining < 0 ? "over-budget" : "",
            text: String(remaining),
          }),
        ]),
      ])
    );
    if ((cls.autotrainable_specs || []).length) {
      panel.appendChild(
        el("p", { class: "hint" }, [
          `Autotrains (free up to level ${maxAutotrainLevel(state.level)} at this character level): ` +
            cls.autotrainable_specs.join(", ") +
            " - already discounted below.",
        ])
      );
    }
    panel.appendChild(
      el("p", { class: "hint" }, ["Click a level number to train straight to it. Hover one to see what it grants."])
    );

    const cap = specCap();
    const rows = el("div", { class: "spec-rows" });
    for (const spec of cls.specs.slice().sort((a, b) => a.name.localeCompare(b.name))) {
      rows.appendChild(renderSpecRow(cls, spec, cap));
    }
    panel.appendChild(
      el("div", { class: "spec-layout" }, [
        rows,
        el("aside", { class: "spec-details", id: "spec-details" }, [
          el("p", { class: "hint", text: "Hover a level number to see what it unlocks." }),
        ]),
      ])
    );
    return panel;
  }

  /** A +/- stepper button - Shift+click steps by 10 instead of 1 (per the owner's own request,
   * useful for jumping a spec/RA level a long way without many individual clicks), clamped to
   * [0, max] either way. */
  function stepButton(label, current, max, onSet) {
    return el("button", {
      text: label,
      disabled: label === "-" ? (current <= 0 ? "disabled" : null) : current >= max ? "disabled" : null,
      onclick: (e) => {
        const delta = (label === "+" ? 1 : -1) * (e.shiftKey ? 10 : 1);
        onSet(Math.max(0, Math.min(max, current + delta)));
      },
    });
  }

  // Ordered, human-labeled field lists for the hover details panel - the same curated columns
  // already exported (see export_charplanner.rs's own SPELL_FIELDS/STYLE_FIELDS), just with
  // friendlier labels. A field whose value is empty/zero/missing is skipped entirely rather than
  // shown as a confusing "0" or blank line - most spells/styles only use a handful of these.
  const SPELL_DETAIL_FIELDS = [
    ["Type", "Type"],
    ["Description", "Description"],
    ["Message1", "Message 1"],
    ["Message2", "Message 2"],
    ["Damage", "Damage"],
    ["DamageType", "Damage Type"],
    ["Value", "Value"],
    ["Range", "Range"],
    ["Radius", "Radius"],
    ["Duration", "Duration"],
    ["Frequency", "Frequency"],
    ["Power", "Power Cost"],
    ["CastTime", "Cast Time"],
    ["RecastDelay", "Recast Delay"],
    ["Target", "Target"],
  ];
  const STYLE_DETAIL_FIELDS = [
    ["EnduranceCost", "Endurance Cost"],
    ["BonusToHit", "Bonus To Hit"],
    ["BonusToDefense", "Bonus To Defense"],
    ["GrowthOffset", "Growth Offset"],
    ["GrowthRate", "Growth Rate"],
    ["WeaponTypeRequirement", "Weapon Type Requirement"],
    ["AttackResultRequirement", "Attack Result Requirement"],
    ["OpeningRequirementType", "Opening Requirement Type"],
    ["OpeningRequirementValue", "Opening Requirement Value"],
    ["StealthRequirement", "Stealth Requirement"],
    ["ArmorHitLocation", "Armor Hit Location"],
  ];

  // eObjectType (Enums/eObjectType.cs) - only the weapon-relevant values are realistically ever
  // seen in a style's own WeaponTypeRequirement, but the full enum is small enough to just
  // include outright rather than silently falling back to a bare number for an unusual one.
  const WEAPON_TYPES = {
    0: "Any",
    1: "Generic Weapon",
    2: "Crushing",
    3: "Slashing",
    4: "Thrusting",
    5: "Fired",
    6: "Two-Handed",
    7: "Polearm",
    8: "Staff",
    9: "Longbow",
    10: "Crossbow",
    11: "Sword",
    12: "Hammer",
    13: "Axe",
    14: "Spear",
    15: "Composite Bow",
    16: "Thrown",
    17: "Left Axe",
    18: "Recurved Bow",
    19: "Blades",
    20: "Blunt",
    21: "Piercing",
    22: "Large Weapons",
    23: "Celtic Spear",
    24: "Flexible",
    25: "Hand to Hand",
    26: "Scythe",
    27: "Fist Wraps",
    28: "Mauler Staff",
    42: "Shield",
    // Style.SpecialWeaponType - not real eObjectType values, two sentinels specific to styles.
    1000: "Dual Wield (both hands)",
    1001: "Any Weapon (1h/2h/dual)",
  };

  // eAttackResultRequirement (styles/Style.cs) - 7 ("Style") is handled separately above as a
  // resolved chain prerequisite, so it never reaches this lookup.
  const ATTACK_RESULT_REQUIREMENTS = {
    0: "Any",
    1: "Miss",
    2: "Hit",
    3: "Parry",
    4: "Block",
    5: "Evade",
    6: "Fumble",
  };

  /** Fills the sticky details panel with everything known about one level's unlock(s) - called
   * on hover, not through the normal render() cycle (a full re-render on every mouse movement
   * would be wasteful and would also fight with the hover itself). More than one unlock can
   * share the same level (rare, but real - e.g. two spells in the same progression); each gets
   * its own heading. `specStyles` (all of this spec's own styles, not just the ones in `group`)
   * is needed to resolve a chain style's own prerequisite - see the AttackResultRequirement===7
   * special case below. */
  function showSpecDetails(group, specStyles) {
    const panel = $("#spec-details");
    if (!panel) return;
    panel.innerHTML = "";
    for (const u of group) {
      panel.appendChild(el("h3", { text: `${u.name} (Lv ${u.level})` }));
      const dl = el("dl");
      const fields = u.kind === "spell" ? SPELL_DETAIL_FIELDS : STYLE_DETAIL_FIELDS;
      // A style whose AttackResultRequirement is 7 ("a style is required" - eAttackResultRequirement.Style,
      // confirmed in the real server source) needs a *prerequisite style*, not a mechanical
      // combat result - its own OpeningRequirementValue is that prerequisite's real Style.ID
      // (a separate field from the DB's own StyleID primary key - see specs.rs's own clone_spec
      // chain-repoint logic for the full citation), not a meaningful number on its own. Resolve
      // it to the prerequisite's actual Name instead of showing a raw, opaque ID - a real,
      // reported case: "Perforate" (ID 214) showing as a bare "214" gave no indication at all
      // that Spear's own "Gungnir's Fury" chains off of it.
      const isChainStyle = u.kind === "style" && Number(u.data.AttackResultRequirement) === 7;
      if (isChainStyle && u.data.OpeningRequirementValue) {
        const prereq = (specStyles || []).find((s) => s.ID === u.data.OpeningRequirementValue);
        dl.appendChild(el("dt", { text: "Requires Style" }));
        dl.appendChild(el("dd", { text: prereq ? prereq.Name : `Unknown (ID ${u.data.OpeningRequirementValue})` }));
      }
      for (const [key, label] of fields) {
        // Already shown above (resolved to a name) - and AttackResultRequirement=7 alone is a
        // meaningless bare "7" without that resolution, so it's skipped here too.
        if (isChainStyle && (key === "OpeningRequirementValue" || key === "AttackResultRequirement")) continue;
        const value = u.data[key];
        if (value === undefined || value === null || value === "" || value === 0) continue;
        let display = String(value);
        if (key === "WeaponTypeRequirement") display = WEAPON_TYPES[value] || display;
        if (key === "AttackResultRequirement") display = ATTACK_RESULT_REQUIREMENTS[value] || display;
        dl.appendChild(el("dt", { text: label }));
        dl.appendChild(el("dd", { text: display }));
      }
      panel.appendChild(dl);
    }
  }

  function renderSpecRow(cls, spec, cap) {
    const current = Math.min(cap, state.specLevels[spec.key_name] || 0);
    state.specLevels[spec.key_name] = current;
    const autotrains = isAutotrainable(cls, spec);
    const setLevel = (v) => {
      state.specLevels[spec.key_name] = v;
      render();
    };

    const row = el("div", { class: "spec-row" });
    const cost = effectiveSpecCost(current, autotrains, state.level);
    const fullCost = specTrainingCost(current);
    const savedText = autotrains && fullCost !== cost ? ` (${fullCost - cost} free via autotrain)` : "";
    row.appendChild(
      el("div", { class: "spec-row-top" }, [
        el("span", { class: "spec-row-name" }, [spec.name, autotrains ? el("span", { class: "badge", text: "autotrains" }) : null]),
        el("div", { class: "stepper" }, [
          stepButton("-", current, cap, setLevel),
          el("span", { class: "value", text: String(current) }),
          stepButton("+", current, cap, setLevel),
        ]),
        el("span", { class: "spec-row-cost", text: `${cost} pt${cost === 1 ? "" : "s"}${savedText}` }),
      ])
    );

    const unlocks = [];
    for (const style of spec.styles) {
      unlocks.push({ level: style.level, name: style.Name, kind: "style", data: style });
    }
    for (const line of spec.spell_lines) {
      for (const spell of line.spells) {
        unlocks.push({ level: spell.level, name: spell.Name, kind: "spell", data: spell });
      }
    }
    unlocks.sort((a, b) => a.level - b.level);

    // Group unlocks sharing the same level into one pip - a real spec line can legitimately have
    // more than one unlock at the exact same level (e.g. two spells in a progression).
    const byLevel = new Map();
    for (const u of unlocks) {
      if (!byLevel.has(u.level)) byLevel.set(u.level, []);
      byLevel.get(u.level).push(u);
    }

    const pips = el("div", { class: "level-pips" });
    for (const [level, group] of [...byLevel.entries()].sort((a, b) => a[0] - b[0])) {
      pips.appendChild(
        el("button", {
          class: `level-pip${level <= current ? " unlocked" : ""}`,
          text: String(level),
          title: group.map((u) => u.name).join(" / "),
          onmouseenter: () => showSpecDetails(group, spec.styles),
          onfocus: () => showSpecDetails(group, spec.styles),
          onclick: () => setLevel(Math.min(cap, level)),
        })
      );
    }
    row.appendChild(pips);
    return row;
  }

  function renderRealmAbilityPanel(cls) {
    const panel = el("div", { class: "panel" });
    panel.appendChild(el("h2", { text: "Realm Abilities" }));
    panel.appendChild(
      el("p", { class: "ra-cost-note" }, [
        "Real per-level training costs from this server's own code, and a real point budget " +
          "from the Realm Rank selected above (AbstractServerRules.GetPlayerRealmPointsTotal).",
      ])
    );

    const totalPoints = totalRaPoints(state.level, realmLevel());
    let totalSpent = 0;
    const table = el("table", { class: "ra-table" }, [
      el("thead", {}, [
        el("tr", {}, [
          el("th", { text: "Ability" }),
          el("th", { text: "Description" }),
          el("th", { text: "Level" }),
          el("th", { text: "Cost so far" }),
        ]),
      ]),
    ]);
    const tbody = el("tbody");
    for (const ra of cls.realm_abilities.slice().sort((a, b) => a.name.localeCompare(b.name))) {
      const maxLevel = ra.max_level || 0;
      const current = Math.min(maxLevel, state.raLevels[ra.key_name] || 0);
      state.raLevels[ra.key_name] = current;
      const setLevel = (v) => {
        state.raLevels[ra.key_name] = v;
        render();
      };
      let costSoFar = null;
      if (ra.costs) {
        costSoFar = 0;
        for (let i = 0; i < current; i++) costSoFar += ra.costs[i] || 0;
        totalSpent += costSoFar;
      }
      const amountText =
        ra.amounts && current > 0 ? ` (currently +${ra.amounts[current - 1]})` : "";
      tbody.appendChild(
        el("tr", {}, [
          el("td", { text: ra.name }),
          el("td", { class: "ra-description", text: ra.description }),
          el("td", {}, [
            el("div", { class: "stepper" }, [
              stepButton("-", current, maxLevel, setLevel),
              el("span", { class: "value", text: `${current}/${maxLevel}` }),
              stepButton("+", current, maxLevel, setLevel),
            ]),
          ]),
          el("td", {
            text: costSoFar === null ? "see in-game trainer" : `${costSoFar}${amountText}`,
          }),
        ])
      );
    }
    table.appendChild(tbody);
    panel.appendChild(table);
    const remaining = totalPoints - totalSpent;
    panel.appendChild(
      el("div", { class: "points-summary" }, [
        el("span", {}, ["Total Realm Ability points: ", el("strong", { text: String(totalPoints) })]),
        el("span", {}, ["Spent: ", el("strong", { text: String(totalSpent) })]),
        el("span", {}, [
          "Remaining: ",
          el("strong", { class: remaining < 0 ? "over-budget" : "", text: String(remaining) }),
        ]),
      ])
    );
    return panel;
  }

  function renderGeneratedFooter() {
    const footer = $("#generated-footer");
    if (!footer || !DATA.generated_at_unix) return;
    const d = new Date(DATA.generated_at_unix * 1000);
    footer.textContent = `Class data generated ${d.toLocaleString()}. Dark Age of Camelot is a trademark of its respective owners.`;
  }

  // window.CHARPLANNER_DATA comes from data/classes.js, a plain <script> tag loaded in
  // planner.html *before* this file - not fetch()'d JSON, so this works identically whether this
  // page is opened by just double-clicking it or uploaded to a real webserver (see
  // export_charplanner.rs's own charplanner_data_path doc comment for the full why).
  if (window.CHARPLANNER_DATA) {
    DATA = window.CHARPLANNER_DATA;
    loadFromUrl();
    if (state.classId == null) {
      const first = classesForRealm(state.realm)[0];
      state.classId = first ? first.id : null;
    }
    render();
    renderGeneratedFooter();
  } else {
    $("#app").innerHTML =
      '<div class="panel"><p class="hint">Could not load class data - data/classes.js is missing ' +
      "or failed to load. Make sure you ran the export tool and uploaded the whole Charplanner " +
      "folder, including data/classes.js, alongside this page.</p></div>";
  }
})();
