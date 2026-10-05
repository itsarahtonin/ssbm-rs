const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const template = fs.readFileSync(path.join(__dirname, 'progress.template.html'), 'utf8');
const counters = template.slice(template.indexOf('const streams = '), template.indexOf('const median = '));
const gates = template.slice(template.indexOf('const streamDone = '), template.indexOf('$("ladder").innerHTML'));
function check(streamCount, frameCount, audioCount, unsynced = false) {
  const D = {
    streams: Array.from({ length: streamCount }, () => ({ done: true, differ: 0 })),
    frames: Array.from({ length: frameCount }, () => ({ frames: [{}], ok: true, worst: { psnr: 'Infinity' } })),
    audio: Array.from({ length: audioCount }, () => ({ lists: 1, differ: 0, synced: !unsynced })),
  };
  return vm.runInNewContext(counters + gates + '\n[streamDone, framesDone, audioDone]', { D });
}
assert.deepEqual(Array.from(check(1, 1, 1)), [false, false, false]);
assert.deepEqual(Array.from(check(40, 41, 41, true)), [true, true, false]);
assert.deepEqual(Array.from(check(40, 41, 41)), [true, true, true]);
assert.equal(Number('Infinity').toFixed(2), 'Infinity');
console.log('Graphics and audio gate regression checks passed');
