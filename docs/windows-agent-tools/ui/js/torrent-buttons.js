return [...document.querySelectorAll('button')]
  .filter(b => (b.getAttribute('aria-label') || '').startsWith('Download with BitTorrent'))
  .slice(0, 20)
  .map(b => {
    let p = b;
    for (let i = 0; i < 5 && p.parentElement; i++) p = p.parentElement;
    return b.getAttribute('aria-label') + ' | ' + ((p.innerText.match(/[\d.]+\s*(kB|MB|GB)/) || [''])[0]);
  });
