(function(){
var $=function(s){return document.querySelector(s)};
var read=function(k){try{return JSON.parse(localStorage.getItem(k)||'{}')}catch(e){return {}}};
var save=function(k,v){try{localStorage.setItem(k,JSON.stringify(v))}catch(e){}};
var root=document.documentElement;
var tree=$('#tree');
var open=read('astrolabe-tree');
var filter=function(){var q=$('#q').value.trim().toLowerCase();tree.classList.toggle('filtering',!!q);tree.querySelectorAll('.f').forEach(function(a){a.classList.toggle('miss',!!q&&a.textContent.toLowerCase().indexOf(q)<0&&(a.title||'').toLowerCase().indexOf(q)<0&&(a.dataset.rel||'').toLowerCase().indexOf(q)<0)});if(q)tree.querySelectorAll('details').forEach(function(d){d.open=true})};
var bindTree=function(){
tree.querySelectorAll('details[data-key]').forEach(function(d){var k=d.dataset.key;if(k in open)d.open=!!open[k];d.addEventListener('toggle',function(){open[k]=d.open;save('astrolabe-tree',open)})});
var hid=$('#hid'),ht=$('#hidtoggle');
if(ht){var show=function(on){hid.hidden=!on;ht.textContent=(on?'Hide ':'Show ')+ht.dataset.names+' · '+ht.dataset.n+' note'+(ht.dataset.n==='1'?'':'s')+' agents wrote for each other'};show(!!read('astrolabe-hidden').on);ht.addEventListener('click',function(){var on=hid.hidden;save('astrolabe-hidden',{on:on});show(on)})}
filter();
};
bindTree();
$('#q').addEventListener('input',filter);
/* The served page follows every save: the server says `changed`, the page reads itself
again and swaps the tree and the board or the note in place, so nothing scrolls or
closes, and a card that changed is marked for a moment. */
if(location.protocol!=='file:'&&window.EventSource){
var live=$('#live');live.hidden=false;live.classList.add('off');
var swap=function(sel,doc){var a=document.querySelector(sel),b=doc.querySelector(sel);if(a&&b)a.innerHTML=b.innerHTML};
var refresh=function(){fetch(location.pathname,{cache:'no-store'}).then(function(r){return r.text()}).then(function(html){
var doc=new DOMParser().parseFromString(html,'text/html');
var before={};document.querySelectorAll('.card[data-n]').forEach(function(c){before[c.dataset.n]=c.innerHTML});
swap('#tree',doc);bindTree();
var board=$('#board');if(board){swap('#board',doc);document.querySelectorAll('.card[data-n]').forEach(function(c){if(before[c.dataset.n]!==c.innerHTML){c.classList.add('flash');setTimeout(function(){c.classList.remove('flash')},1500)}})}
var note=document.querySelector('.note');if(note){var fresh=doc.querySelector('.note');if(fresh&&fresh.innerHTML!==note.innerHTML)note.innerHTML=fresh.innerHTML}
tick();
}).catch(function(){})};
var es=new EventSource('/events');
var opened=false;es.onopen=function(){live.classList.remove('off');if(opened)refresh();opened=true};
es.onerror=function(){live.classList.add('off')};
es.addEventListener('changed',refresh);
}
/* A favicon is a separate image document: copy the current page colors into the
shared vector so it follows palette, appearance and system-theme changes too. */
var updateIcon=function(){var icon=$('.brand-icon').cloneNode(true);var colors=getComputedStyle(root);icon.removeAttribute('class');icon.style.setProperty('--icon-accent',colors.getPropertyValue('--accent').trim());icon.style.setProperty('--icon-detail',colors.getPropertyValue('--ink').trim());$('#favicon').href='data:image/svg+xml,'+encodeURIComponent(new XMLSerializer().serializeToString(icon))};
var look=read('astrolabe-look');
var apply=function(){if(look.palette)root.dataset.palette=look.palette;if(look.appearance)root.dataset.theme=look.appearance;else if(look.appearance==='')root.removeAttribute('data-theme');$('#appearance').value=look.appearance||(root.dataset.theme||'');document.querySelectorAll('.sw').forEach(function(b){b.classList.toggle('on',(root.dataset.palette||'iris')===b.dataset.p)});updateIcon()};
$('#appearance').addEventListener('change',function(e){look.appearance=e.target.value;save('astrolabe-look',look);apply()});
document.querySelectorAll('.sw').forEach(function(b){b.addEventListener('click',function(){look.palette=b.dataset.p;save('astrolabe-look',look);apply()})});
apply();
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change',updateIcon);
var age=function(ms){var s=Math.max(0,ms)/1000;if(s<60)return 'just now';if(s<3600)return Math.floor(s/60)+' min';if(s<86400)return Math.floor(s/3600)+' h';return Math.floor(s/86400)+' d'};
var tick=function(){document.querySelectorAll('.age[data-t]').forEach(function(a){a.textContent=age(Date.now()-(+a.dataset.t))})};
tick();setInterval(tick,60000);
var main=$('#main');
var key='astrolabe-scroll:'+location.pathname;
try{var y=sessionStorage.getItem(key);if(y)main.scrollTop=+y}catch(e){}
main.addEventListener('scroll',function(){try{sessionStorage.setItem(key,main.scrollTop)}catch(e){}});
$('#menu').addEventListener('click',function(){$('#side').classList.add('open');$('#scrim').classList.add('on')});
$('#scrim').addEventListener('click',function(){$('#side').classList.remove('open');$('#scrim').classList.remove('on')});
})();