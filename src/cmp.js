// Volt : réponse automatique aux bandeaux de cookies. À injecter à la création de chaque document, dans tous les cadres.
// Ne lit ni n'envoie aucune donnée de la page ; s'arrête pour de bon après DUREE ms (une seule minuterie, aucun observateur).
(() => {
  const MODE = "refuser"; // "refuser" ou "accepter"
  const EXCLUS = []; // hôtes exclus, ex. ["lemonde.fr"] (couvre aussi les sous-domaines)
  const DUREE = 15000, PAS = 300;

  // self.origin : les cadres about:srcdoc / about:blank héritent de l'origine de leur parent (bandeau AppConsent).
  if (!/^https?:/.test(self.origin) || window.__voltCmp) return;
  // Hôte du site affiché : celui du cadre principal, même depuis un cadre d'un autre domaine.
  let hote = location.hostname;
  try { hote = window.top.location.hostname; } catch (e) {
    const ao = location.ancestorOrigins;
    if (ao && ao.length) try { hote = new URL(ao[ao.length - 1]).hostname; } catch (e2) {}
  }
  if (EXCLUS.some(h => hote === h || hote.endsWith("." + h))) return;
  const accepter = MODE === "accepter";
  // État consultable par Volt (compteur) ; non énumérable.
  const etat = { fait: "", clics: 0, fini: false };
  try { Object.defineProperty(window, "__voltCmp", { value: etat }); } catch (e) { return; }

  // Parenthèses retirées : « J'accepte (c'est gratuit) » -> « j'accepte ».
  const norm = t => (t || "").toLowerCase().normalize("NFKC").replace(/[’`´]/g, "'").replace(/\([^)]*\)/g, " ")
    .replace(/[→›»✓✔×>]/g, " ").replace(/\s+/g, " ").trim().replace(/[.!:]+$/, "");
  const REFUS = new Set(["tout refuser", "refuser tout", "refuser", "je refuse", "refuser les cookies", "refuser tous les cookies",
    "continuer sans accepter", "je n'accepte rien", "rejeter", "tout rejeter", "rejeter tout", "refuser et fermer", "non merci",
    "tout refuser et fermer", "reject all", "reject", "reject all cookies", "reject and close", "reject all and close", "decline",
    "decline all", "deny", "deny all", "refuse all", "i do not agree", "i don't agree", "i do not accept", "disagree", "no thanks",
    "necessary only", "only necessary", "use necessary cookies only", "necessary cookies only", "only necessary cookies",
    "essential cookies only", "only essential cookies", "cookies nécessaires uniquement", "uniquement les cookies nécessaires", "continue without accepting", "alle ablehnen", "ablehnen",
    "nur notwendige", "nur notwendige cookies", "nur essenzielle cookies", "nur erforderliche", "nur erforderliche cookies"]);
  const ACCORD = new Set(["tout accepter", "accepter tout", "accepter", "j'accepte", "j'accepte tout", "oui, j'accepte",
    "accepter et fermer", "accepter & fermer", "accepter et continuer", "accepter les cookies", "tout accepter et fermer",
    "accept all", "accept", "i accept", "accept all cookies", "accept cookies", "accept and continue", "accept & continue",
    "accept and close", "accept all and continue", "consent and continue", "agree and continue", "agree & continue",
    "consent", "i consent", "i agree", "agree", "yes, i agree", "allow all", "allow all cookies", "ok", "alle akzeptieren",
    "akzeptieren", "zustimmen", "zustimmen und weiter", "akzeptieren und weiter", "einverstanden", "alle cookies akzeptieren"]);
  const PAYANT = /abonn|subscri|\babo\b|abo\b|payer|\bpay\b|zahlen|pur-?abo/;
  const SUJET = /cookie|consentement|consent|partenaires|partners|traceurs|datenschutz|einwilligung|privacy|vie privée/;
  // Cadre entier = bandeau quand il vient d'un fournisseur connu (Sourcepoint, AppConsent, Google...).
  const CADRE_CMP = /privacy-mgmt\.com|sp-prod\.net|sourcepoint|consensu\.org|appconsent|sfbx\.io|consent\.google\.|consent\.youtube\.|cmp\.|quantcast|inmobi|trustarc|truste\.com/;
  const CONNUS = "#didomi-host,#didomi-notice,#didomi-popup,#onetrust-banner-sdk,#onetrust-pc-sdk,#CybotCookiebotDialog,#axeptio_overlay," +
    "#tarteaucitronAlertBig,#tarteaucitronRoot .tarteaucitronAlertBigTop,#qc-cmp2-container,.klaro .cookie-notice,.klaro .cookie-modal," +
    "#truste-consent-track,.truste_box_overlay,.cky-consent-container,.cmplz-cookiebanner,#iubenda-cs-banner,.fc-consent-root,#sd-cmp,#cmpbox";
  const GENERIQUES = "dialog[open],[role=dialog],[aria-modal=true],[id*=cookie i],[class*=cookie i],[id*=consent i],[class*=consent i]," +
    "[id*=gdpr i],[class*=gdpr i],[aria-label*=cookie i],[id*=privacy i]";

  const visible = el => !!el && (el.checkVisibility ? el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) : el.offsetParent !== null)
    && el.getBoundingClientRect().width > 0;
  const racines = () => {
    const r = [document];
    for (const h of document.querySelectorAll("#usercentrics-root,#usercentrics-cmp-ui,aside#usercentrics-cmp-ui")) if (h.shadowRoot) r.push(h.shadowRoot);
    return r;
  };
  function bandeaux() {
    if (!document.body) return [];
    if (window !== window.top) {
      // Un cadre entier est un bandeau s'il vient d'un fournisseur connu, ou s'il est court et parle de cookies.
      const t = (document.body.innerText || "").toLowerCase();
      if (CADRE_CMP.test(location.hostname) || (t.length < 8000 && /cookie|consentement|consent|partenaires|partners|traceurs|einwilligung/.test(t)))
        return t ? [document.body] : [];
    }
    const out = [];
    for (const r of racines()) {
      for (const el of r.querySelectorAll(CONNUS)) if (visible(el)) out.push(el);
      for (const el of r.querySelectorAll(GENERIQUES)) {
        if (el === document.body || el === document.documentElement || !visible(el)) continue;
        const t = el.innerText || "";
        if (t.length < 8000 && SUJET.test(t.toLowerCase())) out.push(el);
      }
      if (r !== document) out.push(...[...r.children].filter(visible));
    }
    return out;
  }
  const libelle = b => norm(b.innerText || b.value || b.getAttribute("aria-label") || b.getAttribute("title"));
  const boutons = conts => {
    const s = new Set();
    for (const c of conts) for (const b of c.querySelectorAll("button,a,[role=button],input[type=button],input[type=submit]")) if (visible(b)) s.add(b);
    return [...s];
  };
  // Mur « accepter ou s'abonner » : un bouton payant et aucun refus gratuit.
  const estMur = bs => bs.some(b => PAYANT.test(libelle(b))) && !bs.some(b => REFUS.has(libelle(b)));

  function api() {
    const w = window;
    try {
      const D = w.Didomi;
      if (D && typeof D.setUserDisagreeToAll === "function") {
        if (D.shouldUserStatusBeCollected && !D.shouldUserStatusBeCollected()) return "deja";
        accepter ? D.setUserAgreeToAll() : D.setUserDisagreeToAll(); return "didomi";
      }
      const O = w.OneTrust;
      if (O && typeof O.RejectAll === "function") {
        if (O.IsAlertBoxClosed && O.IsAlertBoxClosed()) return "deja";
        accepter ? O.AllowAll() : O.RejectAll(); return "onetrust";
      }
      const C = w.Cookiebot;
      if (C && typeof C.submitCustomConsent === "function") {
        if (C.hasResponse) return "deja";
        C.submitCustomConsent(accepter, accepter, accepter); return "cookiebot";
      }
      const U = w.__ucCmp;
      if (U && typeof U.denyAllConsents === "function") {
        Promise.resolve(accepter ? U.acceptAllConsents() : U.denyAllConsents())
          .then(() => U.saveConsents && U.saveConsents()).then(() => U.closeCmp && U.closeCmp()).catch(() => {});
        return "usercentrics";
      }
      if (w.UC_UI && typeof w.UC_UI.denyAllConsents === "function") {
        Promise.resolve(accepter ? w.UC_UI.acceptAllConsents() : w.UC_UI.denyAllConsents())
          .then(() => w.UC_UI.closeCMP && w.UC_UI.closeCMP()).catch(() => {});
        return "usercentrics";
      }
      const T = w.tarteaucitron;
      if (T && T.userInterface && typeof T.userInterface.respondAll === "function") { T.userInterface.respondAll(accepter); return "tarteaucitron"; }
      const K = w.klaro;
      if (K && typeof K.getManager === "function") {
        const m = K.getManager();
        if (m && !m.confirmed) { m.changeAll(accepter); m.saveAndApplyConsents("script"); return "klaro"; }
      }
      if (typeof w.performBannerAction === "function") { w.performBannerAction(accepter ? "accept_all" : "reject"); return "cookieyes"; }
    } catch (e) {}
    return "";
  }

  const debut = Date.now();
  let actionA = 0, apiVue = false;
  const finir = () => { etat.fini = true; clearInterval(minuterie); };
  // Scrute jusqu'au bout des DUREE ms : une seconde couche (Utiq, Gimii...) peut suivre la première.
  const minuterie = setInterval(() => {
    if (Date.now() - debut > DUREE) return finir();
    const conts = bandeaux();
    if (!conts.length) return;
    const bs = boutons(conts);
    const cible = accepter ? ACCORD : REFUS;
    const bon = bs.find(x => cible.has(libelle(x)) && !PAYANT.test(libelle(x)));
    // Mode refuser : rien du tout (ni interface, ni clic) tant qu'un refus gratuit n'est pas à l'écran.
    if (!accepter && !bon) { if (estMur(bs) && !etat.fait) etat.fait = "mur"; return; }
    if (actionA && Date.now() - actionA < 1500) return; // laisse le bandeau se fermer
    // 1. Interface officielle du fournisseur (une seule fois par document).
    if (!apiVue) {
      apiVue = true;
      const a = api();
      if (a && a !== "deja") { etat.fait = a; actionA = Date.now(); return; }
      if (a === "deja" && etat.fait !== "mur") etat.fait = etat.fait || "deja";
    }
    // 2. Repli : clic sur un bouton du bandeau, au plus trois fois.
    if (bon && etat.clics < 3) { bon.click(); etat.clics++; if (!etat.fait || etat.fait === "deja" || etat.fait === "mur") etat.fait = "clic"; actionA = Date.now(); }
  }, PAS);
})();
