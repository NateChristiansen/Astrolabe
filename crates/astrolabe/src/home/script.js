(function () {
  'use strict';
  var $ = function (s) {
    return document.querySelector(s)
  };
  var read = function (k) {
    try {
      var v = JSON.parse(localStorage.getItem(k) || '{}');
      return v && typeof v === 'object' && !Array.isArray(v) ? v : {}
    } catch (e) {
      return {}
    }
  };
  var save = function (k, v) {
    try {
      localStorage.setItem(k, JSON.stringify(v))
    } catch (e) {}
  };
  var sessionRead = function (k) {
    try {
      return sessionStorage.getItem(k)
    } catch (e) {
      return null
    }
  };
  var sessionSave = function (k, v) {
    try {
      sessionStorage.setItem(k, v)
    } catch (e) {}
  };
  var root = document.documentElement, app = $('.app'), main = $('#main'), tree = $('#tree');
  var board = !!$('#board'), initialCrumb = $('#crumb').innerHTML;
  // The board's folds, remembered per browser. A list view shows its one section open
  // whatever the viewer folded on the overview, and every other fold, such as the
  // journal's earlier updates inside it, as the viewer left it.
  var sections = read('astrolabe-sections');
  var listViews = { journal: 'today', initiatives: 'initiatives', lessons: 'lessons', drafts: 'drafts' };
  var restoreSections = function () {
    document.querySelectorAll('details[data-persist]').forEach(function (d) {
      if (d.dataset.persist in sections) d.open = !!sections[d.dataset.persist]
    })
  };
  var foldForView = function (view) {
    restoreSections();
    var shown = listViews[view] && document.querySelector('details[data-persist="' + listViews[view] + '"]');
    if (shown) shown.open = true
  };
  var views = ['overview', 'efforts', 'initiatives', 'drafts', 'journal', 'lessons', 'notes'];
  var currentView = board ? 'overview' : 'note';
  var positions = {}, initialized = false;
  var scrollKey = function (view) {
    return 'astrolabe-scroll:' + location.pathname + ':' + view
  };
  var rememberScroll = function () {
    positions[currentView] = main.scrollTop;
    sessionSave(scrollKey(currentView), String(main.scrollTop))
  };
  main.addEventListener('scroll', rememberScroll);
  var narrow = matchMedia('(max-width:720px)');
  var collapsed = narrow.matches || !!read('astrolabe-sidebar').collapsed;
  var setSidebar = function () {
    app.classList.toggle('collapsed', collapsed);
    $('#menu').setAttribute('aria-expanded', String(!collapsed));
    $('#menu').setAttribute('aria-label', collapsed ? 'Expand sidebar' : 'Collapse sidebar');
    var drawer = narrow.matches && !collapsed;
    $('#scrim').hidden = !drawer;
    main.inert = drawer;
  };
  var closeSidebar = function () {
    collapsed = true;
    setSidebar();
    save('astrolabe-sidebar', { collapsed: true })
  };
  $('#menu').addEventListener('click', function () {
    collapsed = !collapsed;
    setSidebar();
    save('astrolabe-sidebar', { collapsed: collapsed });
    if (narrow.matches && !collapsed) $('.navigation a').focus({ preventScroll: true })
  });
  $('#scrim').addEventListener('click', function () {
    closeSidebar();
    $('#menu').focus({ preventScroll: true })
  });
  narrow.addEventListener('change', function () {
    collapsed = narrow.matches || !!read('astrolabe-sidebar').collapsed;
    setSidebar()
  });
  setSidebar();
  var setView = function (view, reset) {
    if (initialized) rememberScroll();
    currentView = view;
    root.dataset.view = view;
    foldForView(view);
    $('#content').hidden = view === 'notes';
    $('#notes-browser').hidden = view !== 'notes';
    document.querySelectorAll('.nav-link').forEach(function (a) {
      if (a.dataset.view === (view === 'note' ? 'notes' : view)) a.setAttribute('aria-current', 'page');
      else a.removeAttribute('aria-current')
    });
    $('#crumb').innerHTML = view === 'note'
      ? initialCrumb
      : '<b>' + ({ overview: 'Overview', efforts: 'Efforts', initiatives: 'Initiatives', drafts: 'Drafts', journal: 'Journal', lessons: 'Lessons', notes: 'Notes' }[view]) + '</b>';
    main.scrollTop = reset ? 0 : (positions[view] ?? Number(sessionRead(scrollKey(view)) || 0));
    if (reset) {
      positions[view] = 0;
      sessionSave(scrollKey(view), '0')
    }
  };
  var viewFromHash = function () {
    var v = location.hash.slice(1);
    if (board && v.startsWith('notes/')) return 'notes';
    return views.includes(v) ? v : (board ? 'overview' : 'note')
  };
  // A breadcrumb folder opens the notes view at that folder: its tree rows open and it scrolls into view.
  var revealFolder = function () {
    var h = location.hash.slice(1);
    if (!board || !h.startsWith('notes/')) return;
    var path;
    try {
      path = decodeURIComponent(h.slice(6))
    } catch (e) {
      return
    }
    openFolder(path)
  };
  var openFolder = function (path) {
    var all = Array.from(tree.querySelectorAll('details[data-key]')), target = null;
    all.forEach(function (d) {
      var k = d.dataset.key;
      if (k === path || path.startsWith(k + '/')) {
        d.open = true;
        if (!target || k.length > target.dataset.key.length) target = d
      }
    });
    // A year has no row of its own; its months stand in for it.
    if (!target || target.dataset.key !== path) all.forEach(function (d) {
      if (d.dataset.key.startsWith(path + '/')) {
        d.open = true;
        if (!target || target.dataset.key.length < path.length) target = d
      }
    });
    if (!target) return;
    var ht = $('#hidtoggle');
    if (target.closest('#hid') && $('#hid').hidden && ht) ht.click();
    var s = target.querySelector('summary');
    s.scrollIntoView({ block: 'center' });
    s.focus({ preventScroll: true });
  };
  window.addEventListener('hashchange', function () {
    setView(viewFromHash(), false);
    revealFolder()
  });
  // Explicit home starts at the top; ordinary Back restores the previous position.
  var resetHome = sessionRead('astrolabe-home-reset') === 'true';
  sessionSave('astrolabe-home-reset', 'false');
  // Don't overwrite a stored position with the fresh document's initial zero.
  currentView = viewFromHash();
  positions[currentView] = Number(sessionRead(scrollKey(currentView)) || sessionRead('astrolabe-scroll:' + location.pathname) || 0);
  setView(currentView, resetHome);
  initialized = true;
  // A note's heading URL takes precedence over the stored reading position.
  if (!views.includes(location.hash.slice(1)) && location.hash) {
    try {
      var heading = document.getElementById(decodeURIComponent(location.hash.slice(1)));
      if (heading) heading.scrollIntoView()
    } catch (e) {}
  }
  window.addEventListener('pageshow', function (e) {
    if (e.persisted) setView(viewFromHash(), false)
  });
  document.querySelectorAll('.nav-link').forEach(function (a) {
    a.addEventListener('click', function (e) {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      if (narrow.matches) closeSidebar();
      if (board || a.dataset.view === 'notes') {
        e.preventDefault();
        history.pushState(null, '', '#' + a.dataset.view);
        setView(a.dataset.view, false);
        main.focus({ preventScroll: true })
      }
    })
  });
  window.addEventListener('popstate', function () {
    setView(viewFromHash(), false)
  });
  $('[data-home]').addEventListener('click', function (e) {
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    if (narrow.matches) closeSidebar();
    if (board) {
      e.preventDefault();
      history.pushState(null, '', '#overview');
      setView('overview', true);
      main.focus({ preventScroll: true })
    } else sessionSave('astrolabe-home-reset', 'true');
  });
  // Note readers return to the view that opened them, including its native history entry.
  document.addEventListener('click', function (e) {
    var link = e.target.closest('a[href]');
    if (!link || e.defaultPrevented || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    var destination = new URL(link.href, location.href);
    if (destination.origin === location.origin && destination.pathname.endsWith('.md') && destination.pathname !== location.pathname) {
      rememberScroll();
      sessionSave('astrolabe-return:' + destination.pathname, JSON.stringify({ href: location.pathname + location.hash, label: currentView }));
    }
  });
  // The way back is part of the note, so a live refresh brings a fresh one to point again.
  var pointBack = function () {
    if (board) return;
    try {
      var previous = JSON.parse(sessionRead('astrolabe-return:' + location.pathname) || 'null');
      var referrer = document.referrer ? new URL(document.referrer) : null;
      if (previous && referrer && referrer.origin === location.origin) {
        var destination = new URL(previous.href, location.href), back = $('.back-link');
        if (back && destination.origin === location.origin && destination.pathname === referrer.pathname) {
          back.href = destination.href;
          back.textContent = '← Back to ' + ({
            overview: 'overview',
            efforts: 'efforts',
            initiatives: 'initiatives',
            drafts: 'drafts',
            journal: 'journal',
            lessons: 'lessons',
            notes: 'notes',
            note: 'previous note'
          }[previous.label] || 'overview');
          back.addEventListener('click', function (e) {
            if (!e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey) {
              e.preventDefault();
              history.back()
            }
          });
        }
      }
    } catch (e) {}
  };
  pointBack();
  // The outline marks the section being read: the last heading above the top of the page.
  var markSection = function () {
    var links = Array.from(document.querySelectorAll('.note .outline a'));
    if (!links.length) return;
    var top = main.getBoundingClientRect().top + $('.topbar').offsetHeight + 40, current = links[0];
    links.forEach(function (a) {
      var h = document.getElementById(decodeURIComponent(a.hash.slice(1)));
      if (h && h.getBoundingClientRect().top <= top) current = a
    });
    // At the end of the page the last section is the one being read.
    if (main.scrollTop + main.clientHeight >= main.scrollHeight - 2) current = links[links.length - 1];
    links.forEach(function (a) {
      if (a === current) a.setAttribute('aria-current', 'true');
      else a.removeAttribute('aria-current')
    });
  };
  var marking = false;
  main.addEventListener('scroll', function () {
    if (marking) return;
    marking = true;
    requestAnimationFrame(function () {
      marking = false;
      markSection()
    })
  });
  markSection();
  var open = read('astrolabe-tree');
  var q = $('#q'), scopes = $('#scopes'), status = $('#search-status');
  q.value = sessionRead('astrolabe-note-query') || '';
  // The search: every word must be in a note's number, title or path, or, from the server,
  // in its text. `hits` is the server's answer for `hitsFor`; `scope` a top folder, or all.
  var scope = sessionRead('astrolabe-note-scope') || '', hidOn = false;
  var hits = {}, hitsFor = null, searching = false, textSearch = location.protocol !== 'file:';
  var words = function () {
    return q.value.trim().toLowerCase().split(/\s+/).filter(Boolean)
  };
  var plural = function (n, one) {
    return n + ' ' + one + (n === 1 ? '' : 's')
  };
  // The snippet as text, each searched word marked.
  var snippet = function (text, ws) {
    var el = document.createElement('span'), lower = text.toLowerCase(), i = 0;
    el.className = 'snip';
    while (i < text.length) {
      var at = -1, len = 0;
      ws.forEach(function (w) {
        var j = lower.indexOf(w, i);
        if (j >= 0 && (at < 0 || j < at || (j === at && w.length > len))) {
          at = j;
          len = w.length
        }
      });
      if (at < 0) break;
      el.appendChild(document.createTextNode(text.slice(i, at)));
      var m = document.createElement('mark');
      m.textContent = text.slice(at, at + len);
      el.appendChild(m);
      i = at + len
    }
    el.appendChild(document.createTextNode(text.slice(i)));
    return el
  };
  var filter = function () {
    var ws = words(), on = ws.length > 0, query = ws.join(' '), count = 0, tucked = 0;
    // A hidden folder's chip shows only while its notes do, and a search kept in one that
    // was since hidden goes back to all.
    var hid = $('#hid'), showHid = hidOn;
    if (hid) hid.hidden = !showHid;
    scopes.querySelectorAll('button[data-tucked]').forEach(function (b) {
      b.hidden = !showHid;
      if (!showHid && b.dataset.scope === scope) scope = ''
    });
    tree.classList.toggle('filtering', on);
    var fresh = hitsFor === query;
    tree.querySelectorAll('.f').forEach(function (a) {
      var old = a.querySelector('.snip');
      if (old) old.remove();
      if (!on) {
        a.classList.remove('miss');
        return
      }
      var rel = a.dataset.rel || '', hay = [a.textContent, a.title, rel].join(' ').toLowerCase();
      var inScope = !scope || rel === scope || rel.startsWith(scope + '/');
      var local = ws.every(function (w) { return hay.includes(w) });
      var text = fresh && Object.hasOwn(hits, rel);
      var match = inScope && (local || text);
      a.classList.toggle('miss', !match);
      if (match && text && !local && hits[rel]) a.appendChild(snippet(hits[rel], ws));
      if (!match) return;
      if (hid && hid.contains(a) && !showHid) tucked++;
      else count++
    });
    // A folder's number is what it holds in this search, and a folder holding none goes.
    tree.querySelectorAll('details[data-key]').forEach(function (d) {
      var n = d.querySelector(':scope>summary .n');
      if (n && n.dataset.total === undefined) n.dataset.total = n.textContent;
      var shown = on ? d.querySelectorAll('.f:not(.miss)').length : 0;
      if (n) n.textContent = on ? shown : n.dataset.total;
      d.classList.toggle('empty', on && shown === 0);
      d.open = on ? true : (d.dataset.key in open ? !!open[d.dataset.key] : d.defaultOpen)
    });
    $('#q-clear').hidden = !q.value;
    $('#q-key').hidden = !!q.value;
    scopes.hidden = !on;
    scopes.querySelectorAll('button').forEach(function (b) {
      b.setAttribute('aria-pressed', String(b.dataset.scope === scope))
    });
    status.hidden = !on;
    status.textContent = '';
    if (on) {
      var line = plural(count, 'note') + ' match';
      if (count === 1) line = '1 note matches';
      if (searching) line += ' · searching note text…';
      else if (!textSearch) line += ' · titles only in this copy';
      else if (!fresh) line += ' · couldn’t search note text';
      else if (more) line += ' · stopped at ' + Object.keys(hits).length + ' text matches, add a word to narrow it';
      status.appendChild(document.createTextNode(line));
      if (tucked) {
        // Showing them takes the link away and brings their chip and rows.
        var link = document.createElement('button');
        link.type = 'button';
        link.className = 'status-more';
        link.textContent = tucked + ' more in ' + ($('#hidtoggle')?.dataset.names || 'hidden folders');
        link.addEventListener('click', function () { $('#hidtoggle').click() });
        status.appendChild(document.createTextNode(' · '));
        status.appendChild(link)
      }
    }
    $('#no-notes').hidden = !on || count > 0 || tucked > 0 || searching;
  };
  // Asks the server for notes whose text holds the words, a moment after typing stops; an
  // answer to a query since changed is dropped. `again` asks once more for the same words
  // after a live refresh, keeping what's shown until the answer comes.
  var seq = 0, timer = null, more = false;
  var lookup = function (again) {
    clearTimeout(timer);
    var query = words().join(' ');
    if (!query || !textSearch) {
      seq++;
      searching = false;
      filter();
      return
    }
    var quiet = again && hitsFor === query;
    if (hitsFor === query && !searching && !again) {
      filter();
      return
    }
    if (!quiet) searching = true;
    filter();
    var mine = ++seq;
    timer = setTimeout(function () {
      fetch('/search?q=' + encodeURIComponent(query), { cache: 'no-store' }).then(function (r) {
        if (!r.ok) throw new Error('Search failed');
        return r.json()
      }).then(function (d) {
        if (mine !== seq) return;
        hits = d && d.hits && typeof d.hits === 'object' ? d.hits : {};
        more = !!(d && d.more);
        hitsFor = query
      }, function () {
        // A failed second ask keeps the first answer.
        if (mine !== seq || quiet) return;
        hits = {};
        hitsFor = null
      }).finally(function () {
        if (mine !== seq) return;
        searching = false;
        filter()
      })
    }, 150)
  };
  var setQuery = function (value) {
    q.value = value;
    sessionSave('astrolabe-note-query', value);
    lookup()
  };
  // A chip for each top folder, so a search can stay in one; a hidden folder's is marked
  // `tucked` and shows only while that folder does.
  var bindScopes = function () {
    var keys = Array.from(tree.querySelectorAll(':scope>details[data-key],:scope>.hid>details[data-key]'));
    if (scope && !keys.some(function (d) { return d.dataset.key === scope })) scope = '';
    scopes.textContent = '';
    [{ key: '', label: 'All' }].concat(keys.map(function (d) {
      return { key: d.dataset.key, label: d.querySelector(':scope>summary .t').textContent, tucked: !!d.closest('#hid') }
    })).forEach(function (s) {
      var b = document.createElement('button');
      b.type = 'button';
      b.className = 'scope';
      b.dataset.scope = s.key;
      if (s.tucked) b.dataset.tucked = '';
      b.textContent = s.label;
      b.addEventListener('click', function () {
        scope = s.key;
        sessionSave('astrolabe-note-scope', scope);
        filter()
      });
      scopes.appendChild(b)
    })
  };
  var bindTree = function () {
    tree.querySelectorAll('details[data-key]').forEach(function (d) {
      if (d.defaultOpen === undefined) d.defaultOpen = d.open;
      if (d.dataset.key in open) d.open = !!open[d.dataset.key]
    });
    var ht = $('#hidtoggle');
    if (ht) {
      var show = function (on) {
        hidOn = on;
        ht.textContent = (on ? 'Hide ' : 'Show ') + ht.dataset.names + ' · ' + ht.dataset.n
          + ' note' + (ht.dataset.n === '1' ? '' : 's') + ' agents wrote for each other'
      };
      show(!!read('astrolabe-hidden').on);
      // A refresh that left the tree as it was keeps the same button: bind it once.
      if (!ht.bound) {
        ht.bound = true;
        ht.addEventListener('click', function () {
          var on = !hidOn;
          save('astrolabe-hidden', { on: on });
          show(on);
          filter()
        })
      }
    }
    bindScopes();
    // Fresh rows may hold fresh text, so a live refresh asks again.
    lookup(true);
    jumpList();
  };
  // The rail beside the tree jumps to each top-level folder.
  var jumpList = function () {
    var jump = $('#jump');
    if (!jump) return;
    jump.querySelectorAll('a').forEach(function (a) { a.remove() });
    tree.querySelectorAll(':scope>details[data-key]').forEach(function (d) {
      var a = document.createElement('a');
      a.href = '#notes/' + encodeURIComponent(d.dataset.key);
      a.className = 'l2';
      a.textContent = d.querySelector('summary .t').textContent;
      a.addEventListener('click', function (e) {
        if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        e.preventDefault();
        openFolder(d.dataset.key)
      });
      jump.appendChild(a)
    });
  };
  $('#collapse-all').addEventListener('click', function () {
    tree.querySelectorAll('details[data-key]').forEach(function (d) { d.open = false });
    main.scrollTo({ top: 0 })
  });
  document.addEventListener('toggle', function (e) {
    var d = e.target;
    if (!d.matches('details')) return;
    if (d.dataset.key && !$('#q').value.trim()) {
      open[d.dataset.key] = d.open;
      save('astrolabe-tree', open)
    }
    // A list view opens its own section itself; that one fold isn't the viewer's choice.
    if (d.dataset.persist && d.dataset.persist !== listViews[currentView]) {
      sections[d.dataset.persist] = d.open;
      save('astrolabe-sections', sections)
    }
  }, true);
  bindTree();
  revealFolder();
  q.addEventListener('input', function () {
    setQuery(q.value)
  });
  $('#q-clear').addEventListener('click', function () {
    setQuery('');
    q.focus()
  });
  // Esc clears, then leaves; Enter opens the first match.
  q.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      if (q.value) setQuery('');
      else q.blur()
    } else if (e.key === 'Enter' && words().length) {
      var first = Array.from(tree.querySelectorAll('.f:not(.miss)')).find(function (a) {
        return !a.closest('[hidden]')
      });
      if (first) {
        e.preventDefault();
        first.click()
      }
    }
  });
  // `/` anywhere on the notes view goes to the search.
  document.addEventListener('keydown', function (e) {
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey || $('#notes-browser').hidden) return;
    var t = e.target;
    if (t.closest && t.closest('input,textarea,select,[contenteditable]')) return;
    e.preventDefault();
    q.focus();
    q.select()
  });
  var paletteNames = { iris: 'Iris', ember: 'Ember', slate: 'Slate', moss: 'Moss', graphite: 'Graphite' };
  var look = read('astrolabe-look');
  if (!Object.hasOwn(paletteNames, look.palette)) delete look.palette;
  if (!['', 'dark', 'light'].includes(look.appearance)) delete look.appearance;
  var system = matchMedia('(prefers-color-scheme: dark)'), picker = $('#palette-picker'), trigger = picker.querySelector('summary');
  var dark = function () {
    return root.dataset.theme === 'dark' || (!root.dataset.theme && system.matches)
  };
  var updateIcon = function () {
    var icon = $('.brand-icon').cloneNode(true), colors = getComputedStyle(root);
    icon.removeAttribute('class');
    icon.style.setProperty('--icon-accent', colors.getPropertyValue('--accent').trim());
    icon.style.setProperty('--icon-detail', colors.getPropertyValue('--ink').trim());
    $('#favicon').href = 'data:image/svg+xml,' + encodeURIComponent(new XMLSerializer().serializeToString(icon))
  };
  var apply = function () {
    if (look.palette) root.dataset.palette = look.palette;
    if (look.appearance) root.dataset.theme = look.appearance;
    else if (look.appearance === '') root.removeAttribute('data-theme');
    var name = paletteNames[root.dataset.palette] || 'Iris';
    $('#palette-name').textContent = name;
    trigger.setAttribute('aria-label', 'Color palette: ' + name);
    document.querySelectorAll('.sw').forEach(function (b) {
      b.setAttribute('aria-pressed', String(root.dataset.palette === b.dataset.p))
    });
    $('#system-appearance').checked = !root.dataset.theme;
    var next = dark() ? 'Light' : 'Dark';
    $('#appearance-toggle').setAttribute('aria-label', 'Switch to ' + next.toLowerCase() + ' appearance');
    $('.appearance-label').textContent = next;
    $('#appearance-icon').textContent = dark() ? '☼' : '☾';
    updateIcon();
  };
  $('#appearance-toggle').addEventListener('click', function () {
    look.appearance = dark() ? 'light' : 'dark';
    save('astrolabe-look', look);
    apply()
  });
  $('#system-appearance').addEventListener('change', function (e) {
    look.appearance = e.target.checked ? '' : (dark() ? 'dark' : 'light');
    save('astrolabe-look', look);
    apply()
  });
  document.querySelectorAll('.sw').forEach(function (b) {
    b.addEventListener('click', function () {
      look.palette = b.dataset.p;
      save('astrolabe-look', look);
      apply();
      picker.open = false;
      trigger.focus({ preventScroll: true })
    })
  });
  document.addEventListener('pointerdown', function (e) {
    if (!picker.contains(e.target)) picker.open = false
  });
  picker.addEventListener('focusout', function (e) {
    if (e.relatedTarget && !picker.contains(e.relatedTarget)) picker.open = false
  });
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Escape') return;
    if (picker.open) {
      picker.open = false;
      trigger.focus({ preventScroll: true });
      e.preventDefault()
    } else if (narrow.matches && !collapsed) {
      closeSidebar();
      $('#menu').focus({ preventScroll: true });
      e.preventDefault()
    }
  });
  apply();
  system.addEventListener('change', apply);
  var age = function (ms) {
    var s = Math.max(0, ms) / 1000;
    return s < 60 ? 'just now' : s < 3600 ? Math.floor(s / 60) + ' min' : s < 86400 ? Math.floor(s / 3600) + ' h' : Math.floor(s / 86400) + ' d'
  };
  var tick = function () {
    document.querySelectorAll('.age[data-t]').forEach(function (a) {
      a.textContent = age(Date.now() - (+a.dataset.t))
    })
  };
  tick();
  setInterval(tick, 60000);
  // Preserve disclosures, position and keyboard focus when live content changes. A failed
  // fetch, parse or swap rejects, so the caller can say the page is out of date.
  var refresh = function () {
    return fetch(location.pathname, { cache: 'no-store' }).then(function (r) {
      if (!r.ok) throw new Error('Refresh failed');
      return r.text()
    }).then(function (html) {
      var doc = new DOMParser().parseFromString(html, 'text/html');
      var y = main.scrollTop, treeY = tree.scrollTop, active = document.activeElement;
      var focus = active && active.closest('#board,#tree,.note') ? {
        id: active.id,
        tag: active.tagName,
        href: active.getAttribute('href'),
        key: active.closest('[data-n]')?.dataset.n,
        detail: active.closest('details')?.dataset.persist,
        treeKey: active.closest('details')?.dataset.key,
        text: active.textContent
      } : null;
      ['#tree', '#board', '.note'].forEach(function (sel) {
        var a = $(sel), b = doc.querySelector(sel);
        if (a && b && a.innerHTML !== b.innerHTML) a.innerHTML = b.innerHTML
      });
      bindTree();
      foldForView(currentView);
      pointBack();
      markSection();
      tick();
      tree.scrollTop = treeY;
      main.scrollTop = y;
      if (focus && !active.isConnected) {
        var replacement = focus.id
          ? document.getElementById(focus.id)
          : Array.from(document.querySelectorAll('#board a,#tree a,#tree summary,.note a,details[data-persist]>summary')).find(function (a) {
            return a.tagName === focus.tag
              && (focus.href ? a.getAttribute('href') === focus.href : (focus.detail || focus.treeKey || a.textContent === focus.text))
              && (!focus.key || a.closest('[data-n]')?.dataset.n === focus.key)
              && (!focus.detail || a.closest('details')?.dataset.persist === focus.detail)
              && (!focus.treeKey || a.closest('details')?.dataset.key === focus.treeKey)
          });
        (replacement || main).focus({ preventScroll: true })
      }
    })
  };
  if (location.protocol !== 'file:' && window.EventSource) {
    var live = $('#live');
    live.hidden = false;
    live.classList.add('off');
    var es = new EventSource('/events'), opened = false, pending = false, queued = false;
    // Whether the stream is open, and whether the last refresh failed. Only a successful
    // refresh clears the warning; the next change or reconnect tries again.
    var connected = false, stale = false;
    var showLive = function () {
      live.classList.toggle('off', !connected);
      live.classList.toggle('stale', connected && stale);
      live.textContent = !connected ? 'Reconnecting…' : stale ? 'Couldn’t refresh · showing older content' : 'Live updates';
    };
    var schedule = function () {
      if (pending) {
        queued = true;
        return
      }
      pending = true;
      refresh().then(function () {
        stale = false
      }, function () {
        stale = true
      }).finally(function () {
        showLive();
        pending = false;
        if (queued) {
          queued = false;
          schedule()
        }
      })
    };
    es.onopen = function () {
      connected = true;
      showLive();
      if (opened) schedule();
      opened = true
    };
    es.onerror = function () {
      connected = false;
      showLive()
    };
    es.addEventListener('changed', schedule);
  }
})();
