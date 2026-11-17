const btns = [...document.querySelectorAll('button[aria-label="Search books"]')];
const info = btns.map(b => ({ vis: !!b.getBoundingClientRect().width, pressed: b.getAttribute('aria-pressed'), expanded: b.getAttribute('aria-expanded') }));
const visible = btns.find(b => b.getBoundingClientRect().width);
if (visible) visible.click();
await new Promise(r => setTimeout(r, 1200));
return {
  info,
  active: document.activeElement.tagName + ' ' + (document.activeElement.getAttribute('aria-label') || ''),
  inputs: [...document.querySelectorAll('input:not([type=checkbox])')].map(i => `${i.getAttribute('aria-label')}|${i.placeholder}|${!!i.getBoundingClientRect().width}`),
  dialogs: [...document.querySelectorAll('dialog[open],[role=dialog],[role=search]')].map(d => d.tagName + ':' + d.innerText.slice(0, 100)),
};
