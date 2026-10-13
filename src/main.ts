import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';

type Obs={id:number,time:string,type:string,selected_model?:string,selected_effort?:string,runtime_model?:string,runtime_effort?:string,provider_model?:string,evidence:string,details?:string,result:string};
type Settings={codex_home:string|null,notify_mismatch:boolean,start_at_login:boolean,theme:'system'|'light'|'dark',initial_scan_days:number,scheduler_enabled:boolean,scheduler_wake_enabled:boolean,scheduler_model:string};
type SettingsForm={home:string,days:string,notify:boolean,login:boolean,theme:Settings['theme'],schedulerEnabled:boolean,schedulerWakeEnabled:boolean,schedulerModel:string};
type ViewName='main'|'verify'|'settings';
const app=document.querySelector<HTMLDivElement>('#app')!;
const value=(a?:string,b?:string)=>`${a||'Unknown'}${b?` / ${b}`:''}`;
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const state:{filter:string,view:ViewName,probeStatus:string,probeInFlight:boolean,verifyModel:string,verifyEffort:string,settingsDraft:Partial<SettingsForm>,schedulerStatus:string,wakeStatus:string}={filter:'all',view:'main',probeStatus:'',probeInFlight:false,verifyModel:'',verifyEffort:'',settingsDraft:{},schedulerStatus:'',wakeStatus:''};
let loading=false;
let reloadPending=false;

function settingsForm(settings:Settings):SettingsForm{
 return {home:settings.codex_home||'',days:String(settings.initial_scan_days),notify:settings.notify_mismatch,login:settings.start_at_login,theme:settings.theme,schedulerEnabled:settings.scheduler_enabled,schedulerWakeEnabled:settings.scheduler_wake_enabled,schedulerModel:settings.scheduler_model};
}
function currentSettingsForm():SettingsForm{
 return {
  home:(document.querySelector('#home') as HTMLInputElement)?.value??'',
  days:(document.querySelector('#days') as HTMLInputElement)?.value??'7',
  notify:(document.querySelector('#notify') as HTMLInputElement)?.checked??false,
  login:(document.querySelector('#login') as HTMLInputElement)?.checked??false,
  theme:((document.querySelector('#theme') as HTMLSelectElement)?.value??'system') as Settings['theme'],
  schedulerEnabled:(document.querySelector('#scheduler-enabled') as HTMLInputElement)?.checked??false,
  schedulerWakeEnabled:(document.querySelector('#scheduler-wake-enabled') as HTMLInputElement)?.checked??false,
  schedulerModel:(document.querySelector('#scheduler-model') as HTMLInputElement)?.value??'gpt-5.6-luna'
 };
}
function setDraft<K extends keyof SettingsForm>(key:K,value:SettingsForm[K]){state.settingsDraft={...state.settingsDraft,[key]:value};}

async function load(){
 if(loading){reloadPending=true;return;}
 loading=true;
 try {
  const [rows,settings,current,status,wake]=await Promise.all([invoke<Obs[]>('history',{filter:state.filter,limit:100,offset:0}),invoke<Settings>('get_settings'),invoke<Obs|null>('current_runtime'),invoke<string>('watcher_status'),invoke<{supported:boolean,installed:boolean,enabled:boolean,owned_events:number,detail:string}>('wake_status')]);
  const form={...settingsForm(settings),...state.settingsDraft};
  document.documentElement.dataset.theme=form.theme;
  app.innerHTML=`<header><div><strong>Codex Runtime Watch</strong><small>Local evidence monitor</small></div><span class="live">● ${esc(status)}</span></header>
  <nav><button data-view="main">Current</button><button data-view="verify">Verify</button><button data-view="settings">Settings</button></nav>
  <main id="main"><h2>Current turn</h2>${current?card(current):'<section class="empty">No Codex turns observed yet.<br><small>Monitoring continues when the sessions folder is available.</small></section>'}
  <div class="title"><h2>Recent history</h2><select id="filter"><option value="all">All</option><option value="mismatches">Mismatches</option><option value="runtime">Runtime</option><option value="probes">Probes</option></select></div>
  <div class="history">${rows.map(row).join('')||'<p class="muted">History is empty.</p>'}</div><button id="clear" class="danger">Clear history</button></main>
  <main id="verify"><h2>Verify backend</h2><p class="notice">Sends a separate minimal <code>hi</code> request to the Codex backend using your existing local login. Credentials stay in Rust and are never displayed or stored by this app.</p><label>Requested model<input id="model" required value="${esc(state.verifyModel)}"></label><label>Reasoning effort<input id="effort" placeholder="low" value="${esc(state.verifyEffort)}"></label><button id="probe" class="primary" ${state.probeInFlight?'disabled':''}>${state.probeInFlight?'Verifying…':'Verify Backend'}</button><p id="probe-status">${esc(state.probeStatus)}</p></main>
  <main id="settings"><h2>Settings</h2><label>Codex home<input id="home" value="${esc(form.home)}" placeholder="Default ~/.codex"></label><label>Initial scan days<input id="days" type="number" min="1" max="365" value="${esc(form.days)}"></label><label class="check"><input id="notify" type="checkbox" ${form.notify?'checked':''}> Notify on mismatch</label><label class="check"><input id="login" type="checkbox" ${form.login?'checked':''}> Start at login</label><label>Theme<select id="theme"><option>system</option><option>light</option><option>dark</option></select></label><section class="card"><h3>5-hour Window Scheduler · macOS</h3><p class="muted">Off by default. When enabled, sends a minimal Codex request only if a 5h window has ended and weekly usage is below 95%. Starts at 07:00; later wakes follow the actual server reset time (90 seconds early), with 12:00 / 17:00 / 22:00 fallback checks. Quiet hours overnight. Automatic wake is optional and needs one-time macOS administrator approval.</p><label class="check"><input id="scheduler-enabled" type="checkbox" ${wake.supported?'':'disabled'} ${form.schedulerEnabled?'checked':''}> Enable automatic window starts</label><label class="check"><input id="scheduler-wake-enabled" type="checkbox" ${wake.supported?'':'disabled'} ${form.schedulerWakeEnabled?'checked':''}> Wake Mac from Sleep (requires installed Wake Manager and AC power)</label><p class="muted">Wake Manager: ${wake.installed?'Installed':'Not installed'} · ${wake.owned_events} scheduled wake(s). ${esc(wake.detail)}</p><div class="folders"><button id="wake-install" type="button" ${wake.supported?'':'disabled'}>Install / update Wake Manager…</button><button id="wake-uninstall" type="button" ${wake.supported&&wake.installed?'':'disabled'}>Remove Wake Manager…</button></div><p id="wake-status">${esc(state.wakeStatus)}</p><label>Low-cost model<input id="scheduler-model" value="${esc(form.schedulerModel)}" spellcheck="false"></label><button id="scheduler-check" type="button">Dry run · no model request</button><p id="scheduler-status">${esc(state.schedulerStatus)}</p></section><button id="save" class="primary">Save settings</button><p id="settings-status"></p><div class="folders"><button id="open-codex">Open Codex data folder</button><button id="open-app">Open app data folder</button></div><small>Version 0.1.0</small></main>`;
  (document.querySelector('#filter') as HTMLSelectElement).value=state.filter;
  (document.querySelector('#theme') as HTMLSelectElement).value=form.theme;
  showView();
  bind(settings);
 } catch(error) {
  app.innerHTML=`<p class="error">Unable to refresh: ${esc(String(error))}</p>`;
 } finally {
  loading=false;
  if(reloadPending){reloadPending=false;void load();}
 }
}
function showView(){document.querySelectorAll('main').forEach(x=>x.hidden=x.id!==state.view);document.querySelectorAll<HTMLElement>('nav button').forEach(x=>x.classList.toggle('active',x.dataset.view===state.view));}
function card(o:Obs){return `<section class="card"><div class="evidence"><div><small>Selected</small><b>${esc(value(o.selected_model,o.selected_effort))}</b></div><div><small>Runtime</small><b>${esc(value(o.runtime_model,o.runtime_effort))}</b></div><div><small>Provider</small><b>${esc(o.provider_model||'Not observed')}</b></div></div><p class="result">${esc(o.result)}</p><small>${new Date(o.time).toLocaleString()} · ${esc(o.evidence)}</small></section>`}
function row(o:Obs){return `<article><div><span class="tag">${o.type}</span><time>${new Date(o.time).toLocaleString()}</time></div><b>${esc(o.result)}</b><small>Selected ${esc(value(o.selected_model,o.selected_effort))} · Runtime ${esc(value(o.runtime_model,o.runtime_effort))} · Provider ${esc(o.provider_model||'Not observed')}</small><div><button data-copy='${esc(JSON.stringify(o))}'>Copy record</button><button data-delete="${o.id}">Delete</button></div></article>`}
function bind(s:Settings){
 document.querySelectorAll<HTMLElement>('nav button').forEach(b=>b.onclick=()=>{state.view=b.dataset.view as ViewName;showView()});
 document.querySelector('#filter')?.addEventListener('change',e=>{state.filter=(e.target as HTMLSelectElement).value;void load()});
 document.querySelectorAll<HTMLElement>('[data-copy]').forEach(b=>b.onclick=()=>void navigator.clipboard.writeText(b.dataset.copy!));
 document.querySelectorAll<HTMLElement>('[data-delete]').forEach(b=>b.onclick=async()=>{try{await invoke('delete_observation',{id:Number(b.dataset.delete)});await load()}catch(e){alert(`Delete failed: ${e}`)}});
 document.querySelector('#clear')?.addEventListener('click',async()=>{if(confirm('Permanently clear observation history?'))try{await invoke('clear_history');await load()}catch(e){alert(`Clear failed: ${e}`)}});
 document.querySelector('#model')?.addEventListener('input',e=>{state.verifyModel=(e.target as HTMLInputElement).value});
 document.querySelector('#effort')?.addEventListener('input',e=>{state.verifyEffort=(e.target as HTMLInputElement).value});
 document.querySelector('#home')?.addEventListener('input',e=>setDraft('home',(e.target as HTMLInputElement).value));
 document.querySelector('#days')?.addEventListener('input',e=>setDraft('days',(e.target as HTMLInputElement).value));
 document.querySelector('#notify')?.addEventListener('change',e=>setDraft('notify',(e.target as HTMLInputElement).checked));
 document.querySelector('#login')?.addEventListener('change',e=>setDraft('login',(e.target as HTMLInputElement).checked));
 document.querySelector('#scheduler-enabled')?.addEventListener('change',e=>setDraft('schedulerEnabled',(e.target as HTMLInputElement).checked));
 document.querySelector('#scheduler-wake-enabled')?.addEventListener('change',e=>setDraft('schedulerWakeEnabled',(e.target as HTMLInputElement).checked));
 document.querySelector('#wake-install')?.addEventListener('click',async()=>{if(!confirm('Install a narrowly scoped root-owned Wake Manager? macOS will request administrator approval. It ONLY schedules owned pmset wake events (06:58/11:58/16:58/21:58); it never reads Codex credentials or sends requests.'))return;state.wakeStatus='Requesting administrator approval…';await load();try{const r=await invoke<{detail:string}>('wake_install');state.wakeStatus=r.detail;}catch(e){state.wakeStatus='Installation failed: '+String(e);}await load();});
 document.querySelector('#wake-uninstall')?.addEventListener('click',async()=>{if(!confirm('Remove the Wake Manager and only its own scheduled wake events? macOS will request administrator approval.'))return;state.wakeStatus='Requesting administrator approval…';await load();try{const r=await invoke<{detail:string}>('wake_uninstall');state.wakeStatus=r.detail;}catch(e){state.wakeStatus='Removal failed: '+String(e);}await load();});
 document.querySelector('#scheduler-model')?.addEventListener('input',e=>setDraft('schedulerModel',(e.target as HTMLInputElement).value));
 document.querySelector('#scheduler-check')?.addEventListener('click',async()=>{state.schedulerStatus='Checking (no model request)…';await load();try{const r=await invoke<{action:string,detail:string,next_reset?:number,weekly_used_percent?:number}>('scheduler_check');state.schedulerStatus=`${r.action}: ${r.detail}${r.next_reset?' · resets '+new Date(r.next_reset*1000).toLocaleString():''}${r.weekly_used_percent==null?'':' · weekly '+r.weekly_used_percent+'%'}`;}catch(e){state.schedulerStatus='Check failed: '+String(e);}await load();});
 document.querySelector('#theme')?.addEventListener('change',e=>setDraft('theme',(e.target as HTMLSelectElement).value as Settings['theme']));
 document.querySelector('#probe')?.addEventListener('click',async()=>{if(state.probeInFlight)return;state.verifyModel=(document.querySelector('#model') as HTMLInputElement).value;state.verifyEffort=(document.querySelector('#effort') as HTMLInputElement).value;state.probeInFlight=true;state.probeStatus='Verifying…';await load();try{const r=await invoke<Obs>('verify_backend',{model:state.verifyModel,effort:state.verifyEffort});state.probeStatus=`${r.result}${r.provider_model?` · Provider ${r.provider_model}`:''}`}catch(e){state.probeStatus=`Probe failed: ${e}`}finally{state.probeInFlight=false;await load()}});
 document.querySelector('#save')?.addEventListener('click',async()=>{const status=document.querySelector('#settings-status')!;const form=currentSettingsForm();const n={...s,codex_home:form.home.trim()||null,initial_scan_days:Number(form.days),notify_mismatch:form.notify,start_at_login:form.login,theme:form.theme,scheduler_enabled:form.schedulerEnabled,scheduler_wake_enabled:form.schedulerWakeEnabled,scheduler_model:form.schedulerModel.trim()};try{await invoke('save_settings',{settings:n});state.settingsDraft={};await load()}catch(e){status.textContent=`Save failed: ${e}`}});
 document.querySelector('#open-codex')?.addEventListener('click',async()=>{try{await invoke('open_folder',{kind:'codex'})}catch(e){alert(`Open failed: ${e}`)}});
 document.querySelector('#open-app')?.addEventListener('click',async()=>{try{await invoke('open_folder',{kind:'app'})}catch(e){alert(`Open failed: ${e}`)}});
}
void load();
const unlistenUpdate=listen('runtime-watch-update',()=>void load());
const unlistenVerify=listen('runtime-watch-open-verify',()=>{state.view='verify';showView()});
window.addEventListener('beforeunload',()=>{void unlistenUpdate.then(f=>f());void unlistenVerify.then(f=>f())},{once:true});
