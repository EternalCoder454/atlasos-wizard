// Tests data/polkit-1/rules.d/50-atlas-wizard.rules with a stub `polkit`.
// Run: node --test tests/helper/polkit-rules.test.mjs   (or: node <file>)
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(
  path.join(here, "../../data/polkit-1/rules.d/50-atlas-wizard.rules"),
  "utf8",
);

const YES = "yes";
const NO = "no";
const FINISH = "net.eterneon.atlas.wizard.finish";

// `markers`: paths that exist. `spawnBroken`: spawn cannot run at all.
function load({ markers = [], spawnBroken = false } = {}) {
  let rule = null;
  const calls = [];
  const polkit = {
    Result: { YES, NO, NOT_HANDLED: "not-handled" },
    addRule(fn) {
      rule = fn;
    },
    // like polkit: returns stdout, throws when the exit status is not 0
    spawn(argv) {
      calls.push(argv);
      if (spawnBroken) throw new Error("cannot run");
      // (arrays from the rule's realm: compare as JSON)
      if (JSON.stringify(argv.slice(0, 3)) !== '["/usr/bin/test","!","-e"]') {
        throw new Error("unexpected program: " + JSON.stringify(argv));
      }
      if (markers.includes(argv[3])) throw new Error("exit status 1");
      return "";
    },
  };
  vm.runInNewContext(source, { polkit });
  assert.equal(typeof rule, "function");
  return { rule, calls };
}

const setup = { user: "atlas-setup", local: true, active: true };
const granted = [...source.matchAll(/^\s*"([a-zA-Z0-9.\-]+\.[a-zA-Z0-9.\-]+)"/gm)]
  .map((m) => m[1])
  .filter((id) => /^(net\.eterneon|org\.freedesktop)\./.test(id));

test("every action in the list is parsed", () => {
  assert.equal(granted.length, 12, granted.join(","));
  assert.ok(granted.includes(FINISH));
});

test("no marker: the setup user is granted every listed action", () => {
  const { rule } = load();
  for (const id of granted) assert.equal(rule({ id }, setup), YES, id);
});

test("any marker: NO for every listed action but the helper's finish", () => {
  for (const marker of ["/etc/atlasos/setup-done", "/etc/plasma-setup-done"]) {
    const { rule } = load({ markers: [marker] });
    for (const id of granted.filter((g) => g !== FINISH)) {
      assert.equal(rule({ id }, setup), NO, `${id} with ${marker}`);
    }
    // finish stays open: EndSetup comes after the markers (see the rule)
    assert.equal(rule({ id: FINISH }, setup), YES);
  }
});

test("both markers present: NO as well", () => {
  const { rule } = load({
    markers: ["/etc/atlasos/setup-done", "/etc/plasma-setup-done"],
  });
  for (const id of granted.filter((g) => g !== FINISH)) {
    assert.equal(rule({ id }, setup), NO, id);
  }
});

test("a broken spawn fails closed", () => {
  const { rule } = load({ spawnBroken: true });
  for (const id of granted.filter((g) => g !== FINISH)) {
    assert.equal(rule({ id }, setup), NO, id);
  }
});

test("other users, remote or inactive sessions, other actions: not handled", () => {
  const { rule, calls } = load();
  const id = granted[0];
  assert.equal(rule({ id }, { ...setup, user: "ada" }), undefined);
  assert.equal(rule({ id }, { ...setup, local: false }), undefined);
  assert.equal(rule({ id }, { ...setup, active: false }), undefined);
  assert.equal(rule({ id: "org.freedesktop.login1.reboot" }, setup), undefined);
  assert.equal(calls.length, 0, "no process is spawned for those");
});
