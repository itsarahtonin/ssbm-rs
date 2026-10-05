const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const template = fs.readFileSync(path.join(__dirname, 'treemap.template.html'), 'utf8');
const logic = template.slice(template.indexOf('function isVerified('), template.indexOf('\nlet C = {};'));
const elements = new Map();
const context = vm.createContext({ showLedger: false, at: 0, fns: [],
  DATA: { fns: [[0, 0, 10]] }, SNAPS: [{ label: 'old', v: [5, 0, 1], countable: [5] }],
  document: { getElementById: id => {
    if (!elements.has(id)) elements.set(id, { setAttribute(name, value) { this[name] = value; } });
    return elements.get(id);
  } },
  d3: { sum: (rows, fn) => rows.reduce((sum, row) => sum + fn(row), 0),
    rollup: (rows, fn, key) => new Map([...new Set(rows.map(key))].map(k => [k, fn(rows.filter(r => key(r) === k))])) },
  fmt: n => String(n), pct: (n, d) => (d ? 100 * n / d : 0).toFixed(1) + '%', blocksAll: 0,
});
vm.runInContext(logic, context);
const check = value => {
  context.fns = [value];
  vm.runInContext('classify()', context);
  return value.status;
};
const crashScreen = { done: 194, countable: 827, explained: 633, calls: 1, bad: 0,
  mutated: 0, reviewed: 0, share: 194 / 827, whole: 1 };
assert.equal(check({ ...crashScreen }), 'low');
context.showLedger = true;
assert.equal(check({ ...crashScreen }), 'low');
const pending = { ...crashScreen, done: 827, explained: 0, share: 1, mutated: 2 };
assert.equal(check(pending), 'review');
assert.equal(vm.runInContext('isVerified(fns[0])', context), false);
assert.equal(check({ ...pending, reviewed: 1 }), 'ok');
assert.equal(check({ ...pending, bad: 1, reviewed: 1 }), 'bad');
assert.equal(check({ ...crashScreen, countable: 0, done: 0, explained: 0 }), 'excluded');
assert.equal(vm.runInContext('SUMS[0].countable', context), 5);
assert.equal(vm.runInContext('SUMS[0].blocks', context), 5);

vm.runInContext(template.slice(template.indexOf('function bar('), template.indexOf('// Treemap')), context);
check({ ...crashScreen });
context.showLedger = false;
vm.runInContext('header()', context);
const stats = elements.get('stats').innerHTML;
assert.match(stats, /23\.5%/);
assert.equal(elements.get('ledger-summary')['aria-hidden'], 'true');
assert.doesNotMatch(elements.get('coverage-bar').innerHTML, /explained/);
context.showLedger = true;
vm.runInContext('header()', context);
assert.equal(elements.get('stats').innerHTML, stats);
assert.equal(elements.get('ledger-summary')['aria-hidden'], 'false');
assert.match(elements.get('ledger-summary').textContent, /100\.0% accounted for/);
assert.match(elements.get('coverage-bar').innerHTML, /explained/);
assert.equal(vm.runInContext('esc("<script> & ")', context), '&lt;script&gt; &amp; ');
console.log('Treemap regression checks passed');
