// Données communes aux 5 maquettes : les vrais réglages de Volt, regroupés.
const ICON={
  jeu:'<rect x="2" y="6" width="20" height="12" rx="6"/><path d="M6 11h4M8 9v4M15 12h.01M18 10h.01"/>',
  general:'<path d="M4 6h16M4 12h16M4 18h16"/><circle cx="9" cy="6" r="2"/><circle cx="15" cy="12" r="2"/><circle cx="8" cy="18" r="2"/>',
  look:'<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor"/>',
  priv:'<path d="M12 3 4 6v6c0 5 3.5 8 8 9 4.5-1 8-4 8-9V6z"/><path d="m9 12 2 2 4-4"/>',
  ext:'<path d="M9 3h6v4h4v6h-4v4H9v-4H5V7h4z"/>',
  keys:'<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  search:'<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  back:'<path d="m15 18-6-6 6-6"/>',
  close:'<path d="M18 6 6 18M6 6l12 12"/>',
  gear:'<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1"/>',
  hist:'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  star:'<path d="m12 2 3.1 6.3 6.9 1-5 4.9 1.2 6.8L12 17.8 5.8 21l1.2-6.8-5-4.9 6.9-1z"/>',
};
const ico=(k,cls="i")=>`<svg class="${cls}" viewBox="0 0 24 24">${ICON[k]}</svg>`;
const VOLTY=`<svg viewBox="0 0 64 64" style="width:100%;height:100%"><path d="M39 0L24 15h8l-4 10 15-15h-8z" fill="var(--a)"/><path d="M22 22h20a14 14 0 0 1 14 14v8a14 14 0 0 1-14 14H22A14 14 0 0 1 8 44v-8a14 14 0 0 1 14-14z" fill="var(--a)"/><g fill="var(--bg)"><path d="M17 37.5a4.5 4.5 0 0 1 9 0v4a4.5 4.5 0 0 1-9 0z"/><path d="M38 37.5a4.5 4.5 0 0 1 9 0v4a4.5 4.5 0 0 1-9 0z"/></g></svg>`;
const SLEEP=["Après 5 min","Après 10 min","Après 20 min","Après 30 min","Après 1 h","Jamais"];
const COLORS=["#8b6cff","#4ea1ff","#3ecf8e","#f06470","#f5a524","#e879c9"];
const KEYS=[["Ctrl T","Nouvel onglet"],["Ctrl W","Fermer l'onglet"],["Ctrl Tab","Onglet suivant"],["Ctrl Maj Tab","Onglet précédent"],["Ctrl L","Aller à la barre d'adresse"],["Ctrl D","Ajouter aux favoris"],["Ctrl H","Historique"],["F5","Actualiser"],["Ctrl ,","Réglages"],["Échap","Fermer les réglages"]];

const SECTIONS=[
  {id:"jeu",name:"Jeu et performances",desc:"Laisser la RAM et le processeur à vos jeux",items:[
    {k:"game_mode",t:"toggle",label:"Mode Jeu",hint:"Les onglets en arrière-plan passent en mémoire minimale",def:false,quick:true},
    {k:"sleep_min",t:"select",label:"Mise en veille des onglets inutilisés",hint:"Libère la RAM, l'onglet se recharge au clic",opts:SLEEP,def:"Après 10 min"},
    {t:"info",label:"Volt réduit = priorité basse",hint:"Automatique : quand vous réduisez Volt, Windows sert votre jeu en premier"},
  ]},
  {id:"general",name:"Général",desc:"Démarrage, recherche, favoris",items:[
    {k:"restore",t:"toggle",label:"Rouvrir les onglets de la dernière session",hint:"Seul le premier onglet se charge, les autres au clic",def:true},
    {k:"home",t:"text",label:"Page d'accueil",hint:"Nouveaux onglets et bouton Accueil",def:"https://www.google.com"},
    {k:"search",t:"select",label:"Moteur de recherche",hint:"Utilisé par la barre d'adresse",opts:["Google","DuckDuckGo","Bing","Brave Search"],def:"Google"},
    {k:"favbar",t:"toggle",label:"Barre de favoris",hint:"Aussi avec la flèche de la barre d'adresse",def:true,quick:true},
  ]},
  {id:"look",name:"Apparence",desc:"Thème, couleur, mode sombre",items:[
    {k:"theme",t:"select",label:"Thème",hint:"Forme des onglets et des menus",opts:["Volt","Classique","Verre","Esport","Pastel","Angulaire"],def:"Volt"},
    {k:"accent",t:"colors",label:"Couleur d'accent",def:COLORS[0]},
    {k:"dark_web",t:"toggle",label:"Thème sombre pour les sites",hint:"Les sites compatibles passent en sombre",def:true},
    {k:"night",t:"toggle",label:"Mode nuit",hint:"Assombrit tous les sites clairs",def:false,restart:true,quick:true},
  ]},
  {id:"priv",name:"Confidentialité",desc:"Pubs, traqueurs, données",items:[
    {k:"adblock",t:"toggle",label:"Bloqueur de pubs et traqueurs",hint:"uBlock Origin Lite : pages plus légères",def:true,quick:true},
    {k:"clear",t:"action",label:"Effacer les données de navigation",hint:"Historique, cookies, cache et données des sites",btn:"Effacer…",danger:true},
  ]},
  {id:"ext",name:"Extensions",desc:"Chrome Web Store",items:[
    {k:"ext",t:"ext",label:"Ajouter une extension",hint:"Collez le lien du Chrome Web Store ou l'ID"},
    {k:"ext_ubol",t:"toggle",label:"uBlock Origin Lite",hint:"Installée par Volt",def:true},
  ]},
  {id:"keys",name:"Raccourcis clavier",desc:"Tout faire sans la souris",items:[{t:"keys",label:"Raccourcis",hint:KEYS.map(k=>k[1]).join(" ")}]},
];
const ALL=SECTIONS.flatMap(s=>s.items.map(it=>({sec:s,it})));
const STATE=Object.fromEntries(ALL.filter(x=>x.it.k&&"def" in x.it).map(x=>[x.it.k,x.it.def]));
const DEF={...STATE};
let needRestart=false;

const norm=s=>(s||"").normalize("NFD").replace(/[̀-ͯ]/g,"").toLowerCase();
const esc=s=>String(s).replace(/[&<>"]/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
function mark(text,q){
  if(!q)return esc(text);
  const i=norm(text).indexOf(norm(q));
  return i<0?esc(text):esc(text.slice(0,i))+`<mark class="hl">${esc(text.slice(i,i+q.length))}</mark>`+esc(text.slice(i+q.length));
}
function find(q){
  const n=norm(q).trim();
  return n?ALL.filter(({sec,it})=>norm(it.label+" "+(it.hint||"")+" "+sec.name).includes(n)):[];
}
function set(k,v){
  STATE[k]=v;
  if(k==="accent")document.documentElement.style.setProperty("--a",v);
  const it=ALL.find(x=>x.it.k===k)?.it;
  if(it?.restart)needRestart=true;
  dispatchEvent(new CustomEvent("volt-change",{detail:k}));
}
const changed=k=>k in DEF&&STATE[k]!==DEF[k];
const summary=s=>{const sel=s.items.filter(i=>i.t==="select").map(i=>STATE[i.k]),on=s.items.filter(i=>i.t==="toggle"&&STATE[i.k]).length;return[...sel,on?`${on} activé${on>1?"s":""}`:""].filter(Boolean).join(" · ")};

// contrôle d'un réglage
function control(it){
  const c=document.createElement("div");c.className="c";
  if(it.t==="toggle"){const i=document.createElement("input");i.type="checkbox";i.className="sw";i.checked=STATE[it.k];i.setAttribute("aria-label",it.label);i.onchange=()=>set(it.k,i.checked);c.append(i)}
  else if(it.t==="select"){const s=document.createElement("select");s.setAttribute("aria-label",it.label);it.opts.forEach(o=>s.append(new Option(o,o)));s.value=STATE[it.k];s.onchange=()=>set(it.k,s.value);c.append(s)}
  else if(it.t==="text"){const i=document.createElement("input");i.type="text";i.value=STATE[it.k];i.setAttribute("aria-label",it.label);i.onchange=()=>set(it.k,i.value);c.append(i)}
  else if(it.t==="colors"){const w=document.createElement("div");w.className="sws";COLORS.forEach(col=>{const b=document.createElement("button");b.style.background=col;b.title=col;b.className=STATE.accent===col?"on":"";b.onclick=()=>{set("accent",col);w.querySelectorAll("button").forEach(x=>x.classList.toggle("on",x===b))};w.append(b)});c.append(w)}
  else if(it.t==="action"){const b=document.createElement("button");b.className="btn"+(it.danger?" bad":"");b.textContent=it.btn;b.onclick=()=>b.textContent="Effacé ✓";c.append(b)}
  else if(it.t==="ext"){c.innerHTML='<input type="text" placeholder="chromewebstore.google.com/detail/…" aria-label="Lien de l\'extension"><button class="btn p">Installer</button>'}
  return c;
}
function row(it,q){
  const r=document.createElement("div");
  if(it.t==="keys"){r.className="keys";r.innerHTML=KEYS.map(([k,l])=>`<span>${mark(l,q)}</span><span>${k.split(" ").map(x=>`<kbd>${x}</kbd>`).join(" ")}</span>`).join("");return r}
  r.className="row";r.dataset.k=it.k||"";
  r.innerHTML=`<div class="l"><b>${mark(it.label,q)}${changed(it.k)?'<i class="mod" title="Modifié"></i>':""}</b>${it.hint?`<small>${mark(it.hint,q)}</small>`:""}</div>`;
  if(it.restart)r.querySelector("b").insertAdjacentHTML("beforeend",' <span class="badge">Redémarrage</span>');
  if(it.t!=="info")r.append(control(it));
  return r;
}
function chrome(addr="volt://settings",tab="Réglages"){
  return`<header class="chrome"><div class="top"><span class="vy">${VOLTY}</span><div class="tab off">YouTube</div><div class="tab">${tab}</div></div>
  <div class="bar"><span class="ib">${ico("back")}</span><div class="addr">${addr}</div><span class="ib">${ico("star")}</span><span class="ib ${STATE.game_mode?"on":""}" id="gamebtn">${ico("jeu")}</span><span class="ib on">${ico("gear")}</span></div></header>`;
}
function restartBar(){return`<div class="restart" id="restart"><span>Le mode nuit s'appliquera au prochain démarrage de Volt.</span><button class="btn p">Redémarrer maintenant</button></div>`}
addEventListener("volt-change",()=>{document.getElementById("restart")?.classList.toggle("on",needRestart);document.getElementById("gamebtn")?.classList.toggle("on",STATE.game_mode)});
