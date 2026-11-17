const out = [];
const walk = (root, where) => {
  for (const el of root.querySelectorAll('*')) {
    const custom = (!el.namespaceURI || el.namespaceURI === 'http://www.w3.org/1999/xhtml') &&
      (el.localName.includes('-') || el.hasAttribute('is'));
    if (custom) out.push({ where, tag: el.localName, is: el.getAttribute('is'), shadow: !!el.shadowRoot,
      defined: !!customElements.get(el.localName), parent: el.parentElement && el.parentElement.localName,
      html: el.outerHTML.slice(0, 200) });
    if (el.shadowRoot) walk(el.shadowRoot, where + '>' + el.localName);
  }
};
walk(document, location.href);
return out.slice(0, 20);
