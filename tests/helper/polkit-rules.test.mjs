// Tests data/polkit-1/rules.d/50-telamon-wizard.rules with a stub `polkit`.
// Run: node --test tests/helper/polkit-rules.test.mjs   (or: node <file>)
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(
  path.join(here, "../../data/polkit-1/rules.d/50-telamon-wizard.rules"),
  "utf8",
);

const YES = "yes";
const NO = "no";
const FINISH = "net.eterneon.telamon.wizard.finish";

// `markers`: paths that exist. `spawnBroken`: spawn cannot run at all.
// `noParent`: parent directories that do not exist (or cannot be checked).
function load({ markers = [], spawnBroken = false, noParent = [] } = {}) {
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
      const shape = [argv[0], argv[1], argv[3], argv[4], argv[6], argv[7], argv[8]];
      if (
        argv.length !== 10 ||
        argv[5] !== argv[2] ||
        JSON.stringify(shape) !== '["/usr/bin/test","-d","-a","-x","-a","!","-e"]'
      ) {
        throw new Error("unexpected program: " + JSON.stringify(argv));
      }
      // `test -d parent -a -x parent -a ! -e marker`: fails when the parent
      // is missing or cannot be searched
      if (noParent.includes(argv[2])) throw new Error("exit status 1");
      if (markers.includes(argv[9])) throw new Error("exit status 1");
      return "";
    },
  };
  vm.runInNewContext(source, { polkit });
  assert.equal(typeof rule, "function");
  return { rule, calls };
}

const setup = { user: "telamon-setup", local: true, active: true };
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
  // ours, Atlas Wizard's (a machine it set up) and plasma-setup's
  for (const marker of [
    "/etc/telamon/setup-done",
    "/etc/atlasos/setup-done",
    "/etc/plasma-setup-done",
  ]) {
    const { rule } = load({ markers: [marker] });
    for (const id of granted.filter((g) => g !== FINISH)) {
      assert.equal(rule({ id }, setup), NO, `${id} with ${marker}`);
    }
    // finish stays open: EndSetup comes after the markers (see the rule)
    assert.equal(rule({ id: FINISH }, setup), YES);
  }
});

test("all markers present: NO as well", () => {
  const { rule } = load({
    markers: [
      "/etc/telamon/setup-done",
      "/etc/atlasos/setup-done",
      "/etc/plasma-setup-done",
    ],
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

test("a missing or unreadable parent directory is NO, not 'no marker'", () => {
  for (const parent of ["/etc/telamon", "/etc/atlasos", "/etc"]) {
    const { rule, calls } = load({ noParent: [parent] });
    for (const id of granted.filter((g) => g !== FINISH)) {
      assert.equal(rule({ id }, setup), NO, id + " " + parent);
    }
    assert.ok(calls.some((c) => c[2] === parent), parent + " is checked");
  }
});

test("each marker is tested with its own parent directory", () => {
  const { rule, calls } = load();
  rule({ id: granted[0] }, setup);
  assert.deepEqual(
    calls.map((c) => [c[2], c[5], c[9]]),
    [
      ["/etc/telamon", "/etc/telamon", "/etc/telamon/setup-done"],
      ["/etc/atlasos", "/etc/atlasos", "/etc/atlasos/setup-done"],
      ["/etc", "/etc", "/etc/plasma-setup-done"],
    ],
  );
});

test("other users, remote or inactive sessions, other actions: not handled", () => {
  const { rule, calls } = load();
  const id = granted[0];
  assert.equal(rule({ id }, { ...setup, user: "ada" }), undefined);
  assert.equal(rule({ id }, { ...setup, local: false }), undefined);
  assert.equal(rule({ id }, { ...setup, active: false }), undefined);
  assert.equal(rule({ id: "org.freedesktop.login1.reboot" }, setup), undefined);
  // Atlas Wizard's setup user is not granted anything by this rule
  assert.equal(rule({ id }, { ...setup, user: "atlas-setup" }), undefined);
  assert.equal(calls.length, 0, "no process is spawned for those");
});
