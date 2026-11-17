const nav = l => [...document.querySelectorAll('button,a')].find(b => (b.getAttribute('aria-label') || b.innerText).trim() === l);
nav('Library').click();
await new Promise(r => setTimeout(r, 1500));
const rows = [...document.querySelectorAll('li, article, [role=row], [role=listitem], .book, [class*=book]')]
  .filter(e => /ESLint/i.test(e.innerText) && e.querySelector('button') && e.innerText.length < 600);
const row = rows.sort((a, b) => a.innerText.length - b.innerText.length)[0];
return {
  count: rows.length,
  text: row ? row.innerText.replace(/\s+/g, ' ').slice(0, 200) : null,
  buttons: row ? [...row.querySelectorAll('button')].map(b => (b.getAttribute('aria-label') || b.title || b.innerText).trim()) : [],
};
