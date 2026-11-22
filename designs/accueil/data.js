// Commun aux 5 maquettes : barre du navigateur, Volty, favoris de démonstration (140 dont 15 hors dossier), navigateur par défaut.
const $=q=>document.querySelector(q), $$=q=>[...document.querySelectorAll(q)];
const P=new URLSearchParams(location.search);
if(P.get("theme"))document.documentElement.dataset.theme=P.get("theme");
const esc=s=>String(s).replace(/[&<>"]/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const I={
  search:'<circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/>',
  gear:'<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1"/>',
  check:'<path d="M20 6 9 17l-5-5"/>',
  ext:'<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  down:'<path d="M12 4v12M6 10l6 6 6-6M5 20h14"/>',
  star:'<path d="m12 2 3.1 6.3 6.9 1-5 4.9 1.2 6.8L12 17.8 5.8 21l1.2-6.8-5-4.9 6.9-1z"/>',
  chev:'<path d="m9 18 6-6-6-6"/>',
  globe:'<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
  key:'<circle cx="8" cy="15" r="4"/><path d="m11 12 9-9M17 6l3 3M14 9l2 2"/>',
  back:'<path d="m15 18-6-6 6-6"/>',
};
const ic=(k,s="")=>`<svg viewBox="0 0 24 24" ${s}>${I[k]}</svg>`;

// ===== Volty : voltySvg recopié tel quel de ui.html =====
const VBOLT='M39 0L24 15h8l-4 10 15-15h-8z',VBODY='M22 22h20a14 14 0 0 1 14 14v8a14 14 0 0 1-14 14H22A14 14 0 0 1 8 44v-8a14 14 0 0 1 14-14z';
const VEYE='M17 37.5a4.5 4.5 0 0 1 9 0v4a4.5 4.5 0 0 1-9 0z',VEYE2='M38 37.5a4.5 4.5 0 0 1 9 0v4a4.5 4.5 0 0 1-9 0z';
const VEX={normal:[VEYE,VEYE2],det:['M17 34.5l9 3.5v3.5a4.5 4.5 0 0 1-9 0z','M47 34.5l-9 3.5v3.5a4.5 4.5 0 0 0 9 0z'],wink:[VEYE,'M36.5 44a6 6 0 0 1 12 0h-3.6a2.4 2.4 0 0 0-4.8 0z']};
function voltySvg(s,gaming){
  const hides=["halloween","noel","nouvelan"].includes(s),band='M3 36v-5a29 27 0 0 1 58 0v5';
  let back="",front="";
  if(gaming)back+=`<path d="${band}" fill="none" stroke="#3d3552" stroke-width="5"/><path class="vrgb" d="${band}" fill="none" stroke="currentColor" stroke-width="1.4"/>`;
  front+=`<g fill="var(--c)" font-weight="700" font-family="system-ui"><text class="z" x="50" y="16" font-size="8">z</text><text class="z z2" x="55" y="12" font-size="10">z</text><text class="z z3" x="60" y="7" font-size="12">Z</text></g><path class="sweat" d="M57 23c3 4 4 6 4 8a4 4 0 0 1-8 0c0-2 1-4 4-8z" fill="#7fd8ff"/><path class="spark" d="${VBOLT}" fill="#ffe14d" transform="translate(-6 10) scale(.38)"/><path class="spark s2" d="${VBOLT}" fill="#ffe14d" transform="translate(52 0) scale(.32)"/>`;
  const eyes=Object.entries(VEX).map(([k,a])=>`<g class="ex ex-${k}">${a.map(d=>`<path class="eye" d="${d}"/>`).join("")}</g>`).join("");
  return`<svg class="vt" viewBox="-6 -14 76 78"><g class="bob">${back}${hides?"":`<path class="bolt" d="${VBOLT}" fill="var(--c)"/>`}<path d="${VBODY}" fill="var(--c)"/><g class="eyes" fill="var(--eye)">${eyes}</g>${front}</g></svg>`;
}
// (les décorations de périodes de voltySvg sont omises ici : la maquette est montrée hors période)

// ===== barre du navigateur, fidèle à ui.html =====
function chrome(tab="Favoris"){
  return `<div id="tabs"><div id="logo" class="open" title="Volty · ouvrir les favoris">${voltySvg("",false)}</div>
  <div id="list"><div class="tab on"><i class="dot"></i><span>${tab}</span><button class="x"><svg viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg></button></div></div>
  <button id="add" title="Nouvel onglet (Ctrl+T)"><svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg></button>
  <div class="wc"><button><svg viewBox="0 0 24 24"><path d="M5 12h14"/></svg></button><button><svg viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="1"/></svg></button><button><svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></svg></button></div></div>
  <div id="bar"><button class="ib"><svg viewBox="0 0 24 24"><path d="m15 18-6-6 6-6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-3-6.7L21 8M21 3v5h-5"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="m3 11 9-8 9 8v10h-6v-6H9v6H3z"/></svg></button>
  <input id="url" spellcheck="false" placeholder="Rechercher ou saisir une adresse">
  <button class="ib"><svg viewBox="0 0 24 24"><path d="m6 15 6-6 6 6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24">${I.star}</svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="M6 11h4M8 9v4M15 12h.01M18 10h.01"/><rect x="2" y="6" width="20" height="12" rx="6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3"/></svg></button></div><div id="toast"></div>`;
}
let tt;function toast(m){const t=$("#toast");t.textContent=m;t.classList.add("on");clearTimeout(tt);tt=setTimeout(()=>t.classList.remove("on"),2200)}

// ===== favoris de démonstration =====
// "Nom|domaine|couleur de marque[|l]" ; l = pas de favicon, l'initiale s'affiche en secours
const GLYPH={youtube:'<path d="M5 3.5v9l8-4.5z"/>',twitch:'<path d="M3 2h11v7l-3 3H8l-2 2v-2H3zM7 5v4h1.5V5zm3.5 0v4H12V5z"/>',spotify:'<path d="M2.5 6.5c4-1.3 8-1 11 .8l-.7 1.2c-2.7-1.6-6.3-1.8-9.9-.7zm1 3c3-.9 6-.7 8.5.6l-.6 1.1c-2.2-1.1-4.8-1.3-7.5-.5z"/>',discord:'<path d="M3 4.5 6 3.5l.5 1h3l.5-1 3 1 1.2 6.5-3.2 2-1-1.5H6l-1 1.5-3.2-2zM6 7.5a1 1 0 1 0 0 2 1 1 0 0 0 0-2zm4 0a1 1 0 1 0 0 2 1 1 0 0 0 0-2z"/>',steam:'<path d="M8 1.5a6.5 6.5 0 1 1-6.3 8.2l2.6 1a2 2 0 0 0 3.9-.7l2.3-1.7a2.5 2.5 0 1 0-2.5-2.5L6.4 8.3a2 2 0 0 0-1.8.3L1.6 7.4A6.5 6.5 0 0 1 8 1.5z"/>',google:'<path d="M8 3a5 5 0 1 0 4.9 6H8V7.2h6.8A7 7 0 1 1 12.9 3.1l-1.4 1.4A5 5 0 0 0 8 3z"/>'};
const FOLDERS=[
 {id:"jeux",name:"Jeux",c:"#2bff88",e:"🎮",f:"Steam|store.steampowered.com|#1b2838|steam;Epic Games|store.epicgames.com|#2a2a2a;GOG|gog.com|#86328a;Battle.net|battle.net|#148eff;Riot Games|riotgames.com|#d13639;Ubisoft Connect|ubisoft.com|#0070ff;EA app|ea.com|#ff4747;Xbox|xbox.com|#107c10;PlayStation|playstation.com|#0070d1;Nintendo|nintendo.fr|#e60012;itch.io|itch.io|#fa5c5c;Humble Bundle|humblebundle.com|#cc2929;Instant Gaming|instant-gaming.com|#ff5400;Rockstar Games|rockstargames.com|#fcaf17;Game Pass|xbox.com/gamepass|#0e7a0d;Eneba|eneba.com|#6f2cff"},
 {id:"stream",name:"Streaming",c:"#ff3b5c",e:"📺",f:"Twitch|twitch.tv|#9146ff|twitch;YouTube|youtube.com|#ff0000|youtube;Kick|kick.com|#53fc18;Netflix|netflix.com|#e50914;Prime Video|primevideo.com|#00a8e1;Disney+|disneyplus.com|#113ccf;Crunchyroll|crunchyroll.com|#f47521;ADN|animationdigitalnetwork.com|#0095ff;Canal+|canalplus.com|#111111;Molotov|molotov.tv|#ff3a5f|l;Arte|arte.tv|#fa481c;Plex|plex.tv|#e5a00d"},
 {id:"commu",name:"Communautés",c:"#5865f2",e:"💬",f:"Discord|discord.com|#5865f2|discord;Reddit|reddit.com|#ff4500;X|x.com|#222222;Instagram|instagram.com|#d62976;Bluesky|bsky.app|#1185fe;Mastodon|mastodon.social|#6364ff;Forums JV|jeuxvideo.com/forums|#fc4c02;Facebook|facebook.com|#1877f2;TikTok|tiktok.com|#25f4ee;Threads|threads.net|#333333;WhatsApp Web|web.whatsapp.com|#25d366;Steam Communauté|steamcommunity.com|#1b2838|steam"},
 {id:"guides",name:"Guides et wikis",c:"#ffb800",e:"",f:"Fandom|fandom.com|#fa005a;Jeuxvideo.com|jeuxvideo.com|#fc4c02;Gamekult|gamekult.com|#e2001a;IGN|ign.com|#bf1313;PC Gamer|pcgamer.com|#c4161c;Nexus Mods|nexusmods.com|#da8e35;HowLongToBeat|howlongtobeat.com|#287fc2;ProtonDB|protondb.com|#f50057;Metacritic|metacritic.com|#ffcc34;SteamDB|steamdb.info|#3b6fb6;Game8|game8.co|#2f6db5|l;Fextralife|fextralife.com|#7a5c2e|l;Wowhead|wowhead.com|#b3122a;Maxroll|maxroll.gg|#e0662a|l;Bulbapedia|bulbapedia.bulbagarden.net|#3a7bd5"},
 {id:"esport",name:"Esport",c:"#00e5ff",e:"🏆",f:"Liquipedia|liquipedia.net|#1f4f8f;HLTV|hltv.org|#2d6da3;VLR.gg|vlr.gg|#ff4655;LoL Esports|lolesports.com|#0bc4e3;OP.GG|op.gg|#5383e8;U.GG|u.gg|#7e5cff;Mobalytics|mobalytics.gg|#ffd100;Blitz|blitz.gg|#ff5859;Tracker.gg|tracker.gg|#ff3c3c;Dotabuff|dotabuff.com|#a3201c;FACEIT|faceit.com|#ff5500;start.gg|start.gg|#3f80ff;Strafe|strafe.com|#1d1d1d|l"},
 {id:"matos",name:"Matériel PC",c:"#ff8a00",e:"",f:"LDLC|ldlc.com|#0a3a7a;Materiel.net|materiel.net|#e3051b;TopAchat|topachat.com|#00a0e3;Amazon|amazon.fr|#ff9900;NVIDIA|nvidia.com|#76b900;AMD|amd.com|#ed1c24;Intel|intel.fr|#0071c5;Corsair|corsair.com|#1a1a1a;Logitech G|logitechg.com|#00b8fc;Razer|razer.com|#44d62c;SteelSeries|steelseries.com|#ff5200;RTINGS|rtings.com|#d6312b;Tom's Hardware|tomshardware.fr|#d1121a;Les Numériques|lesnumeriques.com|#e4312b;PCPartPicker|pcpartpicker.com|#ea6a1f;Back Market|backmarket.fr|#121212;Cdiscount|cdiscount.com|#1d3f8a;Fnac|fnac.com|#e1a925"},
 {id:"musique",name:"Musique",c:"#ff4fd8",e:"🎧",f:"Spotify|open.spotify.com|#1db954|spotify;Deezer|deezer.com|#a238ff;SoundCloud|soundcloud.com|#ff5500;YouTube Music|music.youtube.com|#ff0000|youtube;Bandcamp|bandcamp.com|#1da0c3;Apple Music|music.apple.com|#fa243c;Qobuz|qobuz.com|#0e0e0e|l;Genius|genius.com|#ffff64"},
 {id:"outils",name:"Outils",c:"#7aa2ff",e:"🛠",f:"Gmail|mail.google.com|#ea4335|google;Google Drive|drive.google.com|#1fa463|google;Outlook|outlook.live.com|#0078d4;Notion|notion.so|#2f2f2f;Canva|canva.com|#00c4cc;DeepL|deepl.com|#0f2b46;ChatGPT|chatgpt.com|#10a37f;GitHub|github.com|#24292f;Figma|figma.com|#a259ff;WeTransfer|wetransfer.com|#409fff;Google Agenda|calendar.google.com|#4285f4|google;Google Maps|maps.google.com|#34a853|google;Trello|trello.com|#0079bf;OBS Studio|obsproject.com|#302e31;Imgur|imgur.com|#1bb76e;Photopea|photopea.com|#18a497|l;Speedtest|speedtest.net|#141526"},
 {id:"admin",name:"Admin et banque",c:"#b5b8c6",e:"🏦",f:"impots.gouv|impots.gouv.fr|#000091;ameli|ameli.fr|#0c419a;CAF|caf.fr|#0060a0|l;La Poste|laposte.fr|#ffc827;Free|free.fr|#cd1e25;Orange|orange.fr|#ff7900;Boursorama|boursorama.com|#e5007d;Crédit Agricole|credit-agricole.fr|#006a4e;Revolut|revolut.com|#191c1f;PayPal|paypal.com|#003087;Doctolib|doctolib.fr|#107aca;SNCF Connect|sncf-connect.com|#0c131f;France Travail|francetravail.fr|#1d3a8a;Urssaf|urssaf.fr|#003a7a"},
];
const LOOSE="Google|google.com|#4285f4|google;Wikipédia|fr.wikipedia.org|#636466;Le Monde|lemonde.fr|#121212;Leboncoin|leboncoin.fr|#ec5a13;Météo-France|meteofrance.com|#005aa0;Allociné|allocine.fr|#fecc00;Vinted|vinted.fr|#09b1ba;Pinterest|pinterest.fr|#e60023;Duolingo|duolingo.com|#58cc02;Marmiton|marmiton.org|#ee3a6f;20 Minutes|20minutes.fr|#0068b1;Free Games Finder|freegamesfinder.gg|#3a3a3a|l;Steam Charts|steamcharts.com|#1b2838;Twitter Spaces|x.com/i/spaces|#222222;Prime Gaming|gaming.amazon.com|#9146ff";
const parse=(s,folder)=>s.split(";").map(x=>{const[n,u,b,g]=x.split("|");return{n,u,b,g:g==="l"?"":g,l:g==="l",folder}});
const FAVS=[...FOLDERS.flatMap(d=>(d.items=parse(d.f,d.id))),...parse(LOOSE,"")];
const LOOSE_FAVS=FAVS.filter(f=>!f.folder);
const dirOf=f=>FOLDERS.find(d=>d.id===f.folder);
// texte sombre sur les pastilles claires
const lum=h=>{const n=parseInt(h.slice(1),16);return(.299*(n>>16)+.587*(n>>8&255)+.114*(n&255))/255};
function fav(f,size=""){
  if(f.l)return`<span class="fi l ${size}">${esc(f.n[0].toUpperCase())}</span>`;
  const fg=lum(f.b)>.62?"#111":"#fff";
  return`<span class="fi ${size}" style="--b:${f.b};--f:${fg}">${f.g?`<svg viewBox="0 0 16 16">${GLYPH[f.g]}</svg>`:esc(f.n[0].toUpperCase())}</span>`;
}
const dico=d=>`<span class="di" style="--c:${d.c}">${d.e||""}</span>`;
const norm=s=>s.normalize("NFD").replace(/[̀-ͯ]/g,"").toLowerCase();
const match=(f,q)=>norm(f.n+" "+f.u).includes(norm(q));
const hl=(t,q)=>{if(!q)return esc(t);const i=norm(t).indexOf(norm(q));return i<0?esc(t):esc(t.slice(0,i))+`<mark>${esc(t.slice(i,i+q.length))}</mark>`+esc(t.slice(i+q.length))};
const open=f=>toast(`Ouverture de ${f.u}`);

// ===== apparence : mêmes thèmes et couleurs que ui.html =====
const COLORS=["#9b5cff","#00e5ff","#2bff88","#ff3b5c","#ff8a00","#ff4fd8"];
const THEMES={volt:["Volt","#9b5cff","#0b0c10","#1b1e28"],classique:["Classique","#8ab4f8","#1b1c1e","#323336"],verre:["Verre","#7aa2ff","#0c0e1b","#2a2f4a"],
  esport:["Esport","#00e5ff","#09090e","#1e1e2a"],pastel:["Pastel","#f4a7c6","#1c1923","#2f2a3a"],angulaire:["Angulaire","#ff4655","#0f1923","#1f2e3c"]};
let theme=document.documentElement.dataset.theme||"volt",accent=THEMES[theme][1];
// rend les vignettes de thème et les pastilles d'accent dans root ; un clic applique les variables à toute la maquette
function looks(root){
  const draw=()=>{
    document.documentElement.dataset.theme=theme;document.documentElement.style.setProperty("--a",accent);
    root.querySelector(".themes").innerHTML=Object.entries(THEMES).map(([k,[n,a,bg,sf]])=>`<button class="th ${k===theme?"on":""}" data-t="${k}"><i style="background:${bg}"><u style="background:${sf}"></u><s style="background:${a}"></s></i><span>${n}</span></button>`).join("");
    root.querySelector(".sw-c").innerHTML=[...new Set([THEMES[theme][1],...COLORS])].map(c=>`<button class="${c===accent?"on":""}" data-c="${c}" style="background:${c}" title="${c}"></button>`).join("");
    root.querySelectorAll(".th").forEach(b=>b.onclick=()=>{theme=b.dataset.t;accent=THEMES[theme][1];draw()});
    root.querySelectorAll(".sw-c button").forEach(b=>b.onclick=()=>{accent=b.dataset.c;draw()});
  };draw();
}

// ===== navigateurs détectés =====
const BROWSERS=[{n:"Opera GX",c:"#fa1e4e",k:138},{n:"Chrome",c:"#1a73e8",k:1},{n:"Edge",c:"#0c8484",k:0}];
const brow=b=>`<span class="fi m" style="--b:${b.c}">${b.n[0]}</span>`;
const found=k=>k===0?"Aucun favori trouvé":k===1?"1 favori trouvé":`${k} favoris trouvés`;
// une ligne d'import ; au clic : « Importation… » puis « 138 favoris importés »
const IMPORTED=new Set();
const imported=k=>k>1?`${k} favoris importés`:"1 favori importé";
function importRow(b){
  const d=IMPORTED.has(b.n);
  return`<div class="row imp" data-k="${b.k}" data-n="${b.n}">${brow(b)}<div style="flex:1"><b>${b.n}</b><small>${d?imported(b.k):found(b.k)}</small></div>${d?`<span class="done">${ic("check")}Importé</span>`:b.k?`<button class="btn g">Importer</button>`:""}</div>`;
}
function wireImport(root,done){
  root.querySelectorAll(".imp .btn").forEach(btn=>btn.onclick=()=>{
    const r=btn.closest(".imp"),k=+r.dataset.k;btn.disabled=true;btn.textContent="Importation…";
    setTimeout(()=>{IMPORTED.add(r.dataset.n);r.querySelector("small").textContent=imported(k);
      btn.replaceWith(Object.assign(document.createElement("span"),{className:"done",innerHTML:ic("check")+"Importé"}));done&&done(k)},700);
  });
}

// ===== navigateur par défaut : Volt ouvre la page de Windows, l'utilisateur y confirme =====
let isDefault=P.get("defaut")==="1";
const DEF_NOTE="Windows ouvre ses paramètres : choisissez Volt pour confirmer.";
function defBlock(){
  return isDefault?`<div class="def"><span class="ok">${ic("check")}Volt est votre navigateur par défaut</span></div>`
    :`<div class="def"><button class="btn g defbtn">Faire de Volt le navigateur par défaut</button><small>${DEF_NOTE}</small></div>`;
}
// même chose en forme de ligne de réglage (page de bienvenue)
function defRow(){
  return isDefault?`<div class="row def"><div><b>Navigateur par défaut</b><small>Les liens ouverts depuis Windows, Discord ou vos mails arrivent dans Volt.</small></div><span class="done">${ic("check")}Volt est votre navigateur par défaut</span></div>`
    :`<div class="row def"><div><b>Navigateur par défaut</b><small>${DEF_NOTE}</small></div><button class="btn g defbtn">Faire de Volt le navigateur par défaut</button></div>`;
}
// maquette : on simule le retour de Windows après confirmation
function wireDefault(root,redraw){
  root.querySelectorAll(".defbtn").forEach(b=>b.onclick=()=>{b.disabled=true;b.textContent="En attente de Windows…";
    setTimeout(()=>{isDefault=true;redraw()},1400)});
}
