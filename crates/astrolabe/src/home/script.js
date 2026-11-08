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
  if (!board) {
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
  }
  var open = read('astrolabe-tree'), sections = read('astrolabe-sections');
  $('#q').value = sessionRead('astrolabe-note-query') || '';
  var filter = function () {
    var q = $('#q').value.trim().toLowerCase(), count = 0;
    tree.classList.toggle('filtering', !!q);
    tree.querySelectorAll('.f').forEach(function (a) {
      var miss = !!q && ![a.textContent, a.title || '', a.dataset.rel || ''].some(function (t) {
        return t.toLowerCase().includes(q)
      });
      a.classList.toggle('miss', miss);
      if (!miss && !a.closest('[hidden]')) count++
    });
    tree.querySelectorAll('details').forEach(function (d) {
      d.open = q ? true : (d.dataset.key in open ? !!open[d.dataset.key] : d.defaultOpen)
    });
    $('#no-notes').hidden = !q || count > 0;
  };
  var bindTree = function () {
    tree.querySelectorAll('details[data-key]').forEach(function (d) {
      if (d.defaultOpen === undefined) d.defaultOpen = d.open;
      if (d.dataset.key in open) d.open = !!open[d.dataset.key]
    });
    var hid = $('#hid'), ht = $('#hidtoggle');
    if (ht) {
      var show = function (on) {
        hid.hidden = !on;
        ht.textContent = (on ? 'Hide ' : 'Show ') + ht.dataset.names + ' · ' + ht.dataset.n
          + ' note' + (ht.dataset.n === '1' ? '' : 's') + ' agents wrote for each other'
      };
      show(!!read('astrolabe-hidden').on);
      ht.addEventListener('click', function () {
        var on = hid.hidden;
        save('astrolabe-hidden', { on: on });
        show(on);
        filter()
      })
    }
    filter();
  };
  var restoreSections = function () {
    document.querySelectorAll('details[data-persist]').forEach(function (d) {
      if (d.dataset.persist in sections) d.open = !!sections[d.dataset.persist]
    })
  };
  document.addEventListener('toggle', function (e) {
    var d = e.target;
    if (!d.matches('details')) return;
    if (d.dataset.key && !$('#q').value.trim()) {
      open[d.dataset.key] = d.open;
      save('astrolabe-tree', open)
    }
    if (d.dataset.persist) {
      sections[d.dataset.persist] = d.open;
      save('astrolabe-sections', sections)
    }
  }, true);
  bindTree();
  restoreSections();
  revealFolder();
  $('#q').addEventListener('input', function () {
    sessionSave('astrolabe-note-query', $('#q').value);
    filter()
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
      restoreSections();
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
