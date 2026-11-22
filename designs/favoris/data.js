// Données de démonstration communes aux trois maquettes : 9 dossiers, 14 favoris hors dossier, 138 en tout.
// Favicons = petites pastilles SVG inline aux couleurs des marques (aucune requête réseau).
const P=(bg,inner,r=4)=>"data:image/svg+xml,"+encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="${r}" fill="${bg}"/>${inner}</svg>`);
const T=(c,t,s=10,w=700,f="Segoe UI")=>`<text x="8" y="8" dy=".36em" text-anchor="middle" font-family="${f}" font-size="${s}" font-weight="${w}" fill="${c}">${t}</text>`;
const ICONS={
  "YouTube":P("#ff0033",'<path d="M6.2 4.8v6.4L11.6 8z" fill="#fff"/>'),
  "Twitch":P("#9146ff",'<path d="M4 3.5h8.5v5.8L10 11.8H8l-1.8 1.8v-1.8H4z" fill="#fff"/><path d="M7.4 5.8v2.4M10 5.8v2.4" stroke="#9146ff" stroke-width="1.3"/>'),
  "GitHub":P("#24292f",'<circle cx="8" cy="9" r="4" fill="#fff"/><path d="M4.4 7 4.6 3.6 7.2 5.4M11.6 7l-.2-3.4-2.6 1.8" fill="#fff"/>',8),
  "Reddit":P("#ff4500",'<ellipse cx="8" cy="9.6" rx="4.6" ry="3.1" fill="#fff"/><circle cx="11.4" cy="4" r="1.2" fill="#fff"/><circle cx="6.3" cy="9.3" r=".9" fill="#ff4500"/><circle cx="9.7" cy="9.3" r=".9" fill="#ff4500"/>',8),
  "Steam":P("#1b2838",'<circle cx="10.3" cy="6" r="2.3" fill="none" stroke="#fff" stroke-width="1.3"/><circle cx="5.6" cy="10.6" r="1.7" fill="#fff"/><path d="m5.6 10.6 4.7-4.6" stroke="#fff" stroke-width="1.3"/>',8),
  "Wikipédia":P("#f4f4f4",T("#111","W",10,400,"Georgia"),8),
  "Le Monde":P("#111",T("#fff","M",11,700,"Georgia"),3),
  "Amazon":P("#232f3e",T("#fff","a",11,700)+'<path d="M4.2 12q3.8 2 7.6 0" stroke="#ff9900" stroke-width="1.3" fill="none"/>'),
  "Stack Overflow":P("#f2f2f2",'<path d="M3.6 9.5v3.7h8.8V9.5" stroke="#bcbbbb" stroke-width="1.3" fill="none"/><path d="M5.5 11.4h4.9M5.7 9.4l4.8.9M6.3 7.3l4.4 2M7.4 5.3l3.8 3" stroke="#f48024" stroke-width="1.2"/>'),
  "Discord":P("#5865f2",'<path d="M4 5.5q4-2 8 0l1 5.2q-1.4 1.3-3 1.6l-.6-1q-1.4.4-2.8 0l-.6 1q-1.6-.3-3-1.6z" fill="#fff"/><circle cx="6.4" cy="8.5" r=".9" fill="#5865f2"/><circle cx="9.6" cy="8.5" r=".9" fill="#5865f2"/>'),
  "Netflix":P("#000",T("#e50914","N",12,800),3),
  "Gmail":P("#fff",'<path d="M3 4.5v7.2h2.2V7.4L8 9.6l2.8-2.2v4.3H13V4.5L8 8.3z" fill="#ea4335"/>',3),
  "Crates.io":P("#1f3b2c",'<path d="M8 3.2 12.4 5.6v4.8L8 12.8 3.6 10.4V5.6zM3.6 5.6 8 8l4.4-2.4M8 8v4.8" fill="none" stroke="#ffc832" stroke-width="1.1" stroke-linejoin="round"/>'),
  "Epic Games":P("#2a2a2a",T("#fff","E",10,800)),
  "GOG":P("#86328a",T("#fff","gog",6.5,800),8),
  "itch.io":P("#fa5c5c",T("#fff","i",11,800)),
  "Humble Bundle":P("#cc2929",T("#fff","h",11,800)),
  "Nexus Mods":P("#da8e35",'<circle cx="8" cy="8" r="4" fill="none" stroke="#fff" stroke-width="1.6"/><circle cx="8" cy="8" r="1.4" fill="#fff"/>',8),
  "Speedrun.com":P("#ffb800",'<path d="M9 2.8 4.6 9h3l-.8 4.2L11.4 7h-3z" fill="#1a1a1a"/>'),
  "HLTV":P("#2b6ea4",T("#fff","H",10,800)),
  "Jeuxvideo.com":P("#ff6a00",T("#fff","jv",8,800)),
  "Battle.net":P("#148eff",'<circle cx="8" cy="8" r="4.2" fill="none" stroke="#fff" stroke-width="1.4"/><circle cx="8" cy="8" r="1.5" fill="#fff"/>'),
  "Riot Games":P("#d13639",'<path d="M3.5 4.5 11 3l1.5 7.5-1.8.5-1-3.6-.9 4L6.6 11 5.2 6.7l-.9 4.2-1.2-.3z" fill="#fff"/>'),
  "Lichess":P("#f4f4f4",T("#111","♞",12,400),8),
  "Game Pass":P("#107c10",'<circle cx="8" cy="8" r="4.6" fill="none" stroke="#fff" stroke-width="1.3"/><path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="#fff" stroke-width="1.3"/>',8),
  "Spotify":P("#1ed760",'<path d="M4.5 6.5q4-1.3 7.2.5M5 8.6q3.3-.9 6 .5M5.5 10.6q2.6-.6 4.6.4" stroke="#000" stroke-width="1.2" fill="none" stroke-linecap="round"/>',8),
  "Deezer":P("#a238ff",T("#fff","d",11,800)),
  "Prime Video":P("#00a8e1",T("#fff","p",11,800)),
  "Leboncoin":P("#ec5a13",T("#fff","lbc",6,800)),
  "DeepL":P("#0f2b46",T("#fff","D",10,800)),
  "Bluesky":P("#1185fe",'<path d="M8 8.2Q5.2 3.6 3.6 4.4 3 6.8 5 8.6 3.6 9.6 5.4 11.6 7 12.6 8 9.6 9 12.6 10.6 11.6 12.4 9.6 11 8.6 13 6.8 12.4 4.4 10.8 3.6 8 8.2z" fill="#fff"/>'),
  "Duolingo":P("#58cc02",T("#fff","D",10,800),8)
};
const LONG="Guide complet : optimiser Windows 11 pour le jeu, partie 3 (pilotes, latence, overclocking)";
const ROOT=["YouTube","Twitch","GitHub","Reddit","Steam","Wikipédia","Le Monde","Amazon","Stack Overflow","Discord","Netflix","Gmail",LONG,"Crates.io"];
const FOLDERS=[
  {id:"jeux",name:"Jeux",color:"#e5484d",icon:"🎮",items:["Steam","Epic Games","GOG","itch.io","Humble Bundle","ProtonDB","Nexus Mods","Speedrun.com","HLTV","Liquipedia","Jeuxvideo.com","Battle.net","Riot Games","Lichess","Game Pass","Twitch"]},
  {id:"dev",name:"Dev",color:"#3e9bff",items:["GitHub","Stack Overflow","Crates.io","docs.rs","MDN Web Docs","Rust Book","Tauri","WebView2 Docs","Can I use","regex101","Compiler Explorer","GitLab","npm","Vercel","Cloudflare","Figma","Lucide","Rust Playground","This Week in Rust","Hacker News","DevDocs","Microsoft Learn"]},
  {id:"vid",name:"Vidéos",color:"#ff6b9a",items:["YouTube","Netflix","Twitch","Prime Video","Disney+","Crunchyroll","Arte.tv","France.tv","Vimeo"]},
  {id:"mus",name:"Musique",color:"#2fd17a",icon:"🎧",items:["Spotify","Deezer","SoundCloud","Bandcamp","Radio France","Genius","Last.fm"]},
  {id:"ach",name:"Achats",color:"#f5a524",items:["Amazon","Leboncoin","LDLC","Materiel.net","Fnac","Cdiscount","Vinted","Back Market","Rakuten","AliExpress","Ikea","Decathlon","Idealo","Dealabs"]},
  {id:"act",name:"Actualités",color:"#8b7bff",items:["Le Monde","Libération","Le Figaro","France Info","Mediapart","Courrier international","L'Équipe","Numerama","Les Numériques","Frandroid","Korben","Next","01net","The Verge","Ars Technica","BBC News","Reuters","Ouest-France"]},
  {id:"out",name:"Outils",color:"#22c3d6",items:["Gmail","DeepL","Google Traduction","Google Drive","Proton Mail","Bitwarden","WeTransfer","iLovePDF","Squoosh","Speedtest","Météo-France","Google Maps","Calendrier","Notion","Canva"]},
  {id:"res",name:"Réseaux",color:"#f26d3d",items:["Discord","Reddit","Bluesky","Mastodon","Instagram","LinkedIn","X","WhatsApp Web"]},
  {id:"cours",name:"Cours",color:"#c8a2ff",items:["Wikipédia","OpenClassrooms","Khan Academy","Coursera","MOOC FUN","Duolingo","Exercism","freeCodeCamp","Larousse","Wiktionnaire","Le Robert","Babbel","Udemy","edX","Rust by Example"]}
];
const TOTAL=ROOT.length+FOLDERS.reduce((n,d)=>n+d.items.length,0);
console.assert(TOTAL===138,"138 favoris attendus, "+TOTAL);
const host=n=>n===LONG?"lesnumeriques.com/guide-windows-11-jeu-3":n.toLowerCase().replace(/[^a-z0-9.]+/g,"")+(n.includes(".")?"":".com");

// --- aides DOM ---
const $=s=>document.querySelector(s);
function h(tag,cls,txt){const e=document.createElement(tag);if(cls)e.className=cls;if(txt!=null)e.textContent=txt;return e}
function favIco(name){
  const s=h("span","ico");
  if(ICONS[name]){const i=new Image();i.src=ICONS[name];i.alt="";s.append(i)}
  else{s.classList.add("l");s.textContent=name[0].toUpperCase()}
  return s;
}
const FOLDER_SVG='<svg viewBox="0 0 24 24"><path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4.6l2 2.2h8.4A1.5 1.5 0 0 1 21 8.7v9.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/></svg>';
function dirIco(d){const s=h("span","fd");s.style.setProperty("--c",d.color);if(d.icon)s.textContent=d.icon;else s.innerHTML=FOLDER_SVG;return s}
function go(name){const u=$("#url");if(u)u.value="https://"+host(name)+"/"}
function showFavPage(on){const p=$(".page");if(p)p.classList.toggle("fp",on)}

// --- contexte : onglets + barre d'adresse fidèles à ui.html ---
function chrome(){
  const tabs=[["Steam : Magasin",0],["Liste des favoris importés",1],["YouTube",0]];
  return `<div id="tabs"><div id="logo"><svg viewBox="0 0 24 24"><path d="M13 2 4 14h7l-1 8 9-12h-7z"/></svg></div><div id="list">${tabs.map(([t,on])=>`<div class="tab${on?" on":""}"><i class="dot"></i><span>${t}</span><button class="x"><svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></svg></button></div>`).join("")}</div>
<button id="add"><svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg></button><div class="wc"><button><svg viewBox="0 0 24 24"><path d="M5 12h14"/></svg></button><button><svg viewBox="0 0 24 24"><rect x="5" y="5" width="14" height="14" rx="1"/></svg></button><button><svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6 6 18"/></svg></button></div></div>
<div id="bar"><button class="ib"><svg viewBox="0 0 24 24"><path d="m15 18-6-6 6-6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-3-6.7L21 8M21 3v5h-5"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="m3 11 9-8 9 8v10h-6v-6H9v6H3z"/></svg></button>
<input id="url" spellcheck="false" value="https://volt.local/favoris/import"><button class="ib"><svg viewBox="0 0 24 24"><path d="m6 15 6-6 6 6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="m12 2 3.1 6.3 6.9 1-5 4.9 1.2 6.8L12 17.8 5.8 21l1.2-6.8-5-4.9 6.9-1z"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><path d="M6 11h4M8 9v4M15 12h.01M18 10h.01"/><rect x="2" y="6" width="20" height="12" rx="6"/></svg></button><button class="ib"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1"/></svg></button></div>`;
}
const PAGE=`<i class="hero"></i><i style="width:46%"></i><i style="width:72%"></i><i style="width:64%"></i><i style="width:38%"></i>
<div class="cover"><b>Page de favoris</b><span>Écran plein qui recouvre la page web (dessiné à part). Clic ailleurs dans la barre pour revenir.</span></div>`;

// Monte la maquette : fenêtre vivante + deux états figés (Jeux ouvert, 900 px). mount(el, state) est fourni par la variante.
function scaffold(mount,note){
  document.body.innerHTML=`<div class="win">${chrome()}<div id="live"></div><div class="page">${PAGE}</div></div>
<div class="frozen"><h3>ÉTAT FIGÉ : DOSSIER « JEUX » OUVERT (1280 px)</h3><div class="strip" style="width:1280px" id="f1"></div></div>
<div class="frozen"><h3>ÉTAT FIGÉ : FENÊTRE ÉTROITE (900 px)</h3><div class="strip" style="width:900px" id="f2"></div></div>
<p class="note">${note}</p>`;
  $(".page").onclick=()=>showFavPage(false);
  mount($("#live"),{});
  mount($("#f1"),{open:"jeux"});
  mount($("#f2"),{});
}
