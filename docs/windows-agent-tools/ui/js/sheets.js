return [...document.styleSheets].map(s => {
  let rules;
  try { rules = s.cssRules.length; } catch (e) { rules = 'BLOCKED: ' + e.name; }
  return { href: s.href, owner: s.ownerNode && (s.ownerNode.tagName + (s.ownerNode.id ? '#' + s.ownerNode.id : '')), rules };
}).concat([{ frameOrigin: location.origin }]);
