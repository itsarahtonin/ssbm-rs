const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const template = fs.readFileSync(path.join(__dirname, 'treemap.template.html'), 'utf8');
const logic = template.slice(template.indexOf('function isVerified('), template.indexOf('\nlet C = {};'));
const sums = template.slice(template.indexOf('const SUMS = '), template.indexOf('function isVerified('));
const context = vm.createContext({ BAR: 1, mode: '3b', fns: [{ countable: 10 }], DATA: { bar: 0.9, fns: [[0, 0, 10]] },
  SNAPS: [{ label: 'old', v: [5, 0, 1], countable: [5] },
          { label: 'mutated', v: [10, 0, 5] }] });
vm.runInContext(sums + logic, context);
const check = (value, mode) => {
  context.mode = mode;
  context.BAR = mode === '3b' ? 1 : 0.9;
  context.fns = [value];
  vm.runInContext('classify()', context);
  return value.status;
};
const crashScreen = { done: 194, countable: 827, explained: 633, calls: 1, bad: 0,
  mutated: 0, reviewed: 0, share: 194 / 827, whole: 1 };
assert.equal(check({ ...crashScreen }, '3b'), 'explained');
assert.equal(check({ ...crashScreen }, '3a'), 'low');
const pending = { ...crashScreen, done: 827, explained: 0, share: 1, mutated: 2 };
assert.equal(check(pending, '3b'), 'review');
assert.equal(vm.runInContext('isVerified(fns[0])', context), false);
assert.equal(check({ ...pending, reviewed: 1 }, '3b'), 'ok');
assert.equal(check({ ...pending, bad: 1, reviewed: 1 }, '3a'), 'bad');
assert.equal(vm.runInContext('SUMS[0].verified', context), 1);
assert.equal(vm.runInContext('SUMS[1].verified', context), 0);
assert.equal(vm.runInContext('SUMS[1].complete', context), 0);
console.log('Treemap regression checks passed');
