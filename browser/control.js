(() => {
  if (window.__tvQuickPanelInstalled) return;
  window.__tvQuickPanelInstalled = true;

  const style = document.createElement('style');
  style.textContent = `
    #tv-quick-panel, #tv-quick-panel * { box-sizing:border-box; }
    #tv-quick-panel {
      position:fixed!important; z-index:2147483647!important; top:16px!important; right:18px!important;
      width:286px!important; padding:10px!important; border-radius:22px!important;
      color:#f8f8fa!important; background:rgba(48,49,54,.68)!important;
      border:1px solid rgba(255,255,255,.18)!important;
      box-shadow:0 12px 38px rgba(0,0,0,.34), inset 0 1px rgba(255,255,255,.11)!important;
      -webkit-backdrop-filter:blur(14px) saturate(1.35)!important; backdrop-filter:blur(14px) saturate(1.35)!important;
      font:12px/1.2 system-ui,-apple-system,sans-serif!important; display:none!important;
      user-select:none!important; -webkit-user-select:none!important; pointer-events:auto!important;
    }
    #tv-quick-panel.tv-show { display:block!important; }
    #tv-quick-panel button { border:0!important; color:#fafafd!important; background:rgba(78,79,84,.82)!important;
      border-radius:12px!important; padding:0!important; margin:0!important; cursor:pointer!important;
      font:600 11px/1.1 system-ui,-apple-system,sans-serif!important; display:flex!important;
      align-items:center!important; justify-content:center!important; gap:7px!important; }
    #tv-quick-panel button.tv-selected { background:rgba(119,121,128,.95)!important; outline:1.5px solid rgba(255,255,255,.94)!important; }
    #tv-quick-panel .tv-head {height:18px;display:flex;align-items:center;justify-content:space-between;padding:0 3px 6px;color:#eee;font-size:10px;font-weight:650}
    #tv-quick-panel .tv-net {display:flex;align-items:center;gap:5px;color:#e6e7e9;font-size:9px;font-weight:550}
    #tv-quick-panel .tv-dot {width:5px;height:5px;border-radius:50%;background:#38d66b;box-shadow:0 0 5px #38d66b}
    #tv-quick-panel .tv-grid {display:grid;grid-template-columns:78px 1fr;grid-template-rows:36px 36px 36px;gap:6px}
    #tv-quick-panel .tv-power {grid-column:1;grid-row:1/3;flex-direction:column;gap:4px;font-size:10px!important}
    #tv-quick-panel .tv-wifi {grid-column:2;grid-row:1;justify-content:flex-start!important;padding:0 10px!important}
    #tv-quick-panel .tv-youtube {grid-column:2;grid-row:2;justify-content:flex-start!important;padding:0 10px!important}
    #tv-quick-panel .tv-livetv {grid-column:1;grid-row:3;justify-content:flex-start!important;padding:0 8px!important}
    #tv-quick-panel .tv-clock {grid-column:2;grid-row:3;pointer-events:none!important;justify-content:center!important;font-size:12px!important}
    #tv-quick-panel .tv-icons {display:grid;grid-template-columns:repeat(4,1fr);gap:6px;margin-top:6px}
    #tv-quick-panel .tv-icons button {height:38px!important;border-radius:10px!important}
    #tv-quick-panel svg {width:18px;height:18px;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round;flex:none}
    #tv-quick-panel .tv-power svg {width:28px;height:28px}
    #tv-quick-panel .tv-wifi svg {color:#68b9ff}
    @media (max-width:700px) { #tv-quick-panel { transform:scale(.8);transform-origin:top right; } }
  `;
  document.documentElement.appendChild(style);

  const panel = document.createElement('div');
  panel.id = 'tv-quick-panel';
  panel.innerHTML = `
    <div class="tv-head"><span id="tv-clock">--:--</span><span class="tv-net"><i class="tv-dot"></i>Wi-Fi</span></div>
    <div class="tv-grid">
      <button class="tv-power" data-action="power"><svg viewBox="0 0 24 24"><path d="M12 2v10"/><path d="M6.2 5.7a8 8 0 1 0 11.6 0"/></svg><span>Power Off</span></button>
      <button class="tv-wifi" data-action="settings"><svg viewBox="0 0 24 24"><path d="M2 8.8a15 15 0 0 1 20 0"/><path d="M5.5 12.4a10 10 0 0 1 13 0"/><path d="M9 16a5 5 0 0 1 6 0"/><circle cx="12" cy="20" r="1"/></svg><span>Wi-Fi<br><small>Verbunden</small></span></button>
      <button class="tv-youtube" data-action="youtube"><svg viewBox="0 0 24 24" style="fill:#f33;stroke:none"><rect x="2" y="5" width="20" height="14" rx="4"/><path d="m10 9 6 3-6 3z" style="fill:white"/></svg><span>YouTube TV</span></button>
      <button class="tv-livetv" data-action="livetv"><svg viewBox="0 0 24 24"><rect x="2" y="4" width="20" height="14" rx="2"/><path d="M8 22h8M12 18v4"/></svg><span>Live TV</span></button>
      <button class="tv-clock"><span id="tv-clock-tile">--:--</span></button>
    </div>
    <div class="tv-icons">
      <button data-action="weather" aria-label="Wetter"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg></button>
      <button data-action="settings" aria-label="Einstellungen"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="m19.4 15 .1.1 1.4 1.1-1.4 2.4-1.7-.7a8 8 0 0 1-1.6.9L16 20h-4l-.3-1.8a8 8 0 0 1-1.6-.9l-1.7.7L7 15.6l1.4-1.1a7 7 0 0 1 0-1.8L7 11.6l1.4-2.4 1.7.7a8 8 0 0 1 1.6-.9L12 7h4l.3 1.8a8 8 0 0 1 1.6.9l1.7-.7 1.4 2.4-1.4 1.1a7 7 0 0 1-.2 2.5z"/></svg></button>
      <button data-action="remote" aria-label="Handy-Fernbedienung"><svg viewBox="0 0 24 24"><rect x="6" y="2" width="12" height="20" rx="3"/><path d="M10 5h4M11 18h2"/></svg></button>
      <button data-action="close" aria-label="Schließen"><svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></svg></button>
    </div>`;
  document.documentElement.appendChild(panel);

  const buttons = () => [...panel.querySelectorAll('button[data-action]')];
  let selected = 1;
  const select = i => { const b=buttons(); selected=(i+b.length)%b.length; b.forEach((v,n)=>v.classList.toggle('tv-selected',n===selected)); };
  const hide = () => { panel.classList.remove('tv-show'); };
  const show = () => { panel.classList.add('tv-show'); select(selected); };
  const act = name => {
    if (name === 'close' || name === 'youtube') { hide(); return; }
    hide();
    fetch('http://127.0.0.1:8080/action/'+name,{mode:'cors',cache:'no-store'}).catch(()=>{});
  };
  panel.addEventListener('click',e=>{const b=e.target.closest('button[data-action]');if(b)act(b.dataset.action);});
  const updateClock=()=>{const d=new Date();const s=d.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});document.getElementById('tv-clock').textContent=s;document.getElementById('tv-clock-tile').textContent=s;};
  updateClock(); setInterval(updateClock,15000);

  window.addEventListener('keydown',e=>{
    if(e.key==='F12'){e.preventDefault();e.stopImmediatePropagation();panel.classList.contains('tv-show')?hide():show();return;}
    if(!panel.classList.contains('tv-show')) return;
    if(e.key==='Escape'||e.key==='Backspace'){e.preventDefault();e.stopImmediatePropagation();hide();return;}
    if(['ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(e.key)){
      e.preventDefault();e.stopImmediatePropagation();
      const b=buttons(),cur=b[selected];if(!cur)return;const a=cur.getBoundingClientRect();const ax=a.left+a.width/2,ay=a.top+a.height/2;
      let best=-1,score=Infinity;
      b.forEach((v,i)=>{if(i===selected)return;const r=v.getBoundingClientRect(),dx=r.left+r.width/2-ax,dy=r.top+r.height/2-ay;
        const ok=e.key==='ArrowLeft'?dx< -1:e.key==='ArrowRight'?dx>1:e.key==='ArrowUp'?dy< -1:dy>1;
        if(ok){const primary=e.key==='ArrowLeft'||e.key==='ArrowRight'?Math.abs(dx):Math.abs(dy);const secondary=e.key==='ArrowLeft'||e.key==='ArrowRight'?Math.abs(dy):Math.abs(dx);const s=primary+secondary*1.5;if(s<score){score=s;best=i;}}});
      if(best>=0)select(best);return;
    }
    if(e.key==='Enter'||e.key===' '){e.preventDefault();e.stopImmediatePropagation();buttons()[selected]?.click();}
  },true);
})();
