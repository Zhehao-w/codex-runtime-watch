import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';

type Obs={id:number,time:string,type:string,selected_model?:string,selected_effort?:string,runtime_model?:string,runtime_effort?:string,provider_model?:string,evidence:string,details?:string,result:string};
type Settings={codex_home:string|null,notify_mismatch:boolean,start_at_login:boolean,theme:'system'|'light'|'dark',initial_scan_days:number};
type ViewName='main'|'verify'|'settings';
const app=document.querySelector<HTMLDivElement>('#app')!;
const value=(a?:string,b?:string)=>`${a||'Unknown'}${b?` / ${b}`:''}`;
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const state:{filter:string,view:ViewName,probeStatus:string,probeInFlight:boolean}={filter:'all',view:'main',probeStatus:'',probeInFlight:false};
let loading=false;
async function load(){
 if(loading)return; loading=true;
 try {
  const [rows,settings,current,status]=await Promise.all([invoke<Obs[]>('history',{filter:state.filter,limit:100,offset:0}),invoke<Settings>('get_settings'),invoke<Obs|null>('current_runtime'),invoke<string>('watcher_status')]);
  document.documentElement.dataset.theme=settings.theme;
  app.innerHTML=`<header><div><strong>Codex Runtime Watch</strong><small>Local evidence monitor</small></div><span class="live">● ${esc(status)}</span></header>
  <nav><button data-view="main">Current</button><button data-view="verify">Verify</button><button data-view="settings">Settings</button></nav>
  <main id="main"><h2>Current turn</h2>${current?card(current):'<section class="empty">No Codex turns observed yet.<br><small>Monitoring continues when the sessions folder is available.</small></section>'}
  <div class="title"><h2>Recent history</h2><select id="filter"><option value="all">All</option><option value="mismatches">Mismatches</option><option value="runtime">Runtime</option><option value="probes">Probes</option></select></div>
  <div class="history">${rows.map(row).join('')||'<p class="muted">History is empty.</p>'}</div><button id="clear" class="danger">Clear history</button></main>
  <main id="verify"><h2>Verify backend</h2><p class="notice">Sends a separate minimal <code>hi</code> request to the Codex backend using your existing local login. Credentials stay in Rust and are never displayed or stored by this app.</p><label>Requested model<input id="model" required></label><label>Reasoning effort<input id="effort" placeholder="low"></label><button id="probe" class="primary" ${state.probeInFlight?'disabled':''}>${state.probeInFlight?'Verifying…':'Verify Backend'}</button><p id="probe-status">${esc(state.probeStatus)}</p></main>
  <main id="settings"><h2>Settings</h2><label>Codex home<input id="home" value="${esc(settings.codex_home||'')}" placeholder="Default ~/.codex"></label><label>Initial scan days<input id="days" type="number" min="1" max="365" value="${settings.initial_scan_days}"></label><label class="check"><input id="notify" type="checkbox" ${settings.notify_mismatch?'checked':''}> Notify on mismatch</label><label class="check"><input id="login" type="checkbox" ${settings.start_at_login?'checked':''}> Start at login</label><label>Theme<select id="theme"><option>system</option><option>light</option><option>dark</option></select></label><button id="save" class="primary">Save settings</button><p id="settings-status"></p><div class="folders"><button id="open-codex">Open Codex data folder</button><button id="open-app">Open app data folder</button></div><small>Version 0.1.0</small></main>`;
  (document.querySelector('#filter') as HTMLSelectElement).value=state.filter;(document.querySelector('#theme') as HTMLSelectElement).value=settings.theme;showView();bind(settings);
 } catch(error) { app.innerHTML=`<p class="error">Unable to refresh: ${esc(String(error))}</p>`; }
 finally { loading=false; }
}
function showView(){document.querySelectorAll('main').forEach(x=>x.hidden=x.id!==state.view);document.querySelectorAll<HTMLElement>('nav button').forEach(x=>x.classList.toggle('active',x.dataset.view===state.view));}
function card(o:Obs){return `<section class="card"><div class="evidence"><div><small>Selected</small><b>${esc(value(o.selected_model,o.selected_effort))}</b></div><div><small>Runtime</small><b>${esc(value(o.runtime_model,o.runtime_effort))}</b></div><div><small>Provider</small><b>${esc(o.provider_model||'Not observed')}</b></div></div><p class="result">${esc(o.result)}</p><small>${new Date(o.time).toLocaleString()} · ${esc(o.evidence)}</small></section>`}
function row(o:Obs){return `<article><div><span class="tag">${o.type}</span><time>${new Date(o.time).toLocaleString()}</time></div><b>${esc(o.result)}</b><small>Selected ${esc(value(o.selected_model,o.selected_effort))} · Runtime ${esc(value(o.runtime_model,o.runtime_effort))} · Provider ${esc(o.provider_model||'Not observed')}</small><div><button data-copy='${esc(JSON.stringify(o))}'>Copy record</button><button data-delete="${o.id}">Delete</button></div></article>`}
function bind(s:Settings){document.querySelectorAll<HTMLElement>('nav button').forEach(b=>b.onclick=()=>{state.view=b.dataset.view as ViewName;showView()});
 document.querySelector('#filter')?.addEventListener('change',e=>{state.filter=(e.target as HTMLSelectElement).value;void load()});
 document.querySelectorAll<HTMLElement>('[data-copy]').forEach(b=>b.onclick=()=>void navigator.clipboard.writeText(b.dataset.copy!));document.querySelectorAll<HTMLElement>('[data-delete]').forEach(b=>b.onclick=async()=>{try{await invoke('delete_observation',{id:Number(b.dataset.delete)});await load()}catch(e){alert(`Delete failed: ${e}`)}});
 document.querySelector('#clear')?.addEventListener('click',async()=>{if(confirm('Permanently clear observation history?'))try{await invoke('clear_history');await load()}catch(e){alert(`Clear failed: ${e}`)}});
 document.querySelector('#probe')?.addEventListener('click',async()=>{if(state.probeInFlight)return;const model=(document.querySelector('#model') as HTMLInputElement).value;const effort=(document.querySelector('#effort') as HTMLInputElement).value;state.probeInFlight=true;state.probeStatus='Verifying…';await load();try{const r=await invoke<Obs>('verify_backend',{model,effort});state.probeStatus=`${r.result}${r.provider_model?` · Provider ${r.provider_model}`:''}`}catch(e){state.probeStatus=`Probe failed: ${e}`}finally{state.probeInFlight=false;await load()}});
 document.querySelector('#save')?.addEventListener('click',async()=>{const status=document.querySelector('#settings-status')!;const n={...s,codex_home:(document.querySelector('#home') as HTMLInputElement).value.trim()||null,initial_scan_days:Number((document.querySelector('#days') as HTMLInputElement).value),notify_mismatch:(document.querySelector('#notify') as HTMLInputElement).checked,start_at_login:(document.querySelector('#login') as HTMLInputElement).checked,theme:(document.querySelector('#theme') as HTMLSelectElement).value};try{await invoke('save_settings',{settings:n});await load()}catch(e){status.textContent=`Save failed: ${e}`}});
 document.querySelector('#open-codex')?.addEventListener('click',async()=>{try{await invoke('open_folder',{kind:'codex'})}catch(e){alert(`Open failed: ${e}`)}});document.querySelector('#open-app')?.addEventListener('click',async()=>{try{await invoke('open_folder',{kind:'app'})}catch(e){alert(`Open failed: ${e}`)}});}
void load();
const unlistenUpdate=listen('runtime-watch-update',()=>void load());
const unlistenVerify=listen('runtime-watch-open-verify',()=>{state.view='verify';showView()});
window.addEventListener('beforeunload',()=>{void unlistenUpdate.then(f=>f());void unlistenVerify.then(f=>f())},{once:true});
