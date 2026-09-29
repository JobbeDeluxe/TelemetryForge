// Regression test for the actual editor preview request functions.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(require('node:path').join(__dirname, '../frontend/app-v2.js'), 'utf8');
const extract = (start, end) => source.slice(source.indexOf(start), source.indexOf(end));
const nodes = { preview: { src: 'initial' }, error: { textContent: '' } };
const requests = [];
const context = vm.createContext({
  config: { widgets: [{ label_format: 'first' }] }, previewRevision: 0,
  previewTimer: null, previewInteractionActive: false, previewRequestBusy: false,
  previewRefreshPending: false, $: id => nodes[id],
  invoke: (command, args) => new Promise(resolve => requests.push({ command, args, resolve })),
  setTimeout: () => 1, clearTimeout: () => {}, requestAnimationFrame: () => {}, renderOverlay: () => {},
});
vm.runInContext(extract('function scheduleLivePreview()', 'async function refreshGifPreview()') +
  extract('async function refreshPreview()', 'async function refreshStatus()'), context);
(async () => {
  const old = context.refreshPreview();
  context.config.widgets[0].label_format = 'new draft';
  context.scheduleLivePreview();
  requests[0].resolve('old image');
  await old;
  assert.equal(nodes.preview.src, 'initial', 'an old response must not replace the current draft');
  assert.equal(requests[0].args.config.widgets[0].label_format, 'first', 'request takes an immutable snapshot');
  const current = context.refreshPreview();
  assert.equal(requests[1].command, 'preview_config');
  assert.equal(requests[1].args.config.widgets[0].label_format, 'new draft');
  requests[1].resolve('new image');
  await current;
  assert.equal(nodes.preview.src, 'new image');
  console.log('Preview regression passed: stale responses discarded, draft preserved.');
})().catch(error => { console.error(error); process.exitCode = 1; });
