# Volt : appliquer la logique « codec vidéo » à la RAM et au CPU d'un navigateur

Recherche du 22/11/2026. Contexte : Volt (Rust + wry/WebView2), runtime WebView2 relevé sur la machine de dev : **157.0.4341.60**. Windows 11, distribution grand public sans dépendance.

## Comment lire ce document

- **[MESURÉ]** : chiffre publié, avec sa source. Le chiffre vaut pour le contexte de la source (Chrome ou Edge, une version donnée, leur banc de test). Il n'est pas transposable tel quel à Volt.
- **[HYPOTHÈSE]** : raisonnement ou estimation de ma part, sans mesure publiée. À valider par A/B.
- **[DOC]** : comportement décrit par la documentation officielle, sans chiffre.

Ce que Volt fait déjà (lu dans `src/perf.rs` et `src/main.rs`, sans rien modifier) :

- `--renderer-process-limit` quand la RAM est ≤ 8 Go ou en Mode Jeu ;
- `IsVisible=false` sur les onglets inactifs ;
- `TrySuspend` après 120 s (30 s en Mode Jeu, 20 s si la fenêtre est réduite) ;
- déchargement complet (`wv = None`) après `sleep_min`, 20 min par défaut ;
- `MemoryUsageTargetLevel=Low` quand la fenêtre est réduite et, en Mode Jeu, sur les onglets inactifs ;
- EcoQoS et `BELOW_NORMAL` sur tout l'arbre de processus quand la fenêtre est réduite ;
- les onglets qui jouent du son ne sont jamais gelés.

La base est donc déjà bonne. Ce document dit où il reste du gain et où il n'y en a pas.

---

## 0. L'analogie codec, posée rigoureusement

Un codec comme H.265 économise sur trois axes :

| Codec | Équivalent dans un navigateur | Ce qui existe vraiment |
|---|---|---|
| **Redondance spatiale ou inter-flux** (même bloc codé une seule fois) | Même code ou mêmes données chargés N fois par N processus | Pages d'images exécutables partagées par l'OS, builtins V8 embarqués, cache de code V8 sur disque, **regroupement des onglets du même site dans un même processus** |
| **Image clé + deltas** | État complet (« keyframe ») + différence propre à l'onglet | Snapshot de démarrage V8 (keyframe commune à tous les isolates), BFCache (snapshot complet d'une page) ; pour un onglet déchargé : la keyframe est le cache HTTP et le cache de code, le delta est URL + défilement + formulaire + position média |
| **Niveaux de qualité / débit adaptatif** | Moins de calcul et de mémoire pour ce qui est peu visible | Throttling des timers, gel, piste vidéo coupée en arrière-plan, plus aucune image produite quand l'onglet est caché, compression et mise en veille mémoire par l'OS |

Constat central pour « 50 onglets YouTube » : la vraie déduplication ne vient pas de l'OS (Page Combining, voir §1). Elle vient de **faire partager un même processus de rendu aux onglets du même site** (§3.1). Dans un même processus, un seul isolate V8 garde en cache le code compilé de YouTube, et le cache mémoire de Blink est commun. Entre processus, seul ce qui est mappé depuis un fichier, comme les DLL ou les builtins embarqués, est partagé gratuitement.

---

## 1. Déduplication mémoire entre processus (KSM, Page Combining)

**Principe.** Le noyau parcourt la RAM, repère les pages de contenu identique et n'en garde qu'une seule copie en copie-sur-écriture. KSM sous Linux a été conçu pour KVM ; Windows a l'équivalent depuis Windows 8 (Page Combining).

**Chiffres et faits publiés**
- [DOC] Windows 8 : Page Combining activé par défaut côté client, désactivé par défaut sur Windows Server 2012. Le coût principal annoncé est un CPU plus élevé. Aucun chiffre de coût n'est publié. Sources : [blog Microsoft, Clint Huffman](https://learn.microsoft.com/en-us/archive/blogs/clinth/memory-combining-in-windows-8-and-windows-server-2012), [Cache and Memory Manager Improvements](https://learn.microsoft.com/en-us/windows-server/administration/performance-tuning/subsystem/cache-memory-management/improvements-in-windows-server).
- [MESURÉ, sécurité] *Dedup Est Machina* (IEEE S&P 2016, VUSec) : la déduplication combinée à Rowhammer permet de compromettre Edge depuis JavaScript sous Windows 10, toutes défenses actives, sans bug logiciel du navigateur. Microsoft a réagi en **désactivant la déduplication par défaut sur Windows 10** (CVE-2016-3272). Source : [vusec.net/projects/dedup-est-machina](https://www.vusec.net/projects/dedup-est-machina/). Le bulletin [MS16-092](https://technet.microsoft.com/library/security/MS16-092) ne mentionne pas explicitement la déduplication. Le lien avec ce CVE repose uniquement sur VUSec.
- [MESURÉ, sécurité] *Dedup Est Machina Returns* (VUSec, 2022) : depuis Windows 10 1903, seules les pages jugées « sûres » sont fusionnées (pages remplies de 0x00 ou de 0xFF) et seulement à l'intérieur d'un même domaine de sécurité. Une fuite reste démontrée dans certains scénarios. Mitigations développeur : `IsolateSecurityDomain` et `DisablePageCombine`. Source : [vusec.net/projects/dedup-est-machina-returns](https://www.vusec.net/projects/dedup-est-machina-returns/).
- [DOC] `PROCESS_MITIGATION_SIDE_CHANNEL_ISOLATION_POLICY.DisablePageCombine` : désactive toute fusion pour un processus, sauf les pages entièrement à 0 ou à 1. Une fois activé, ce réglage ne peut plus être retiré. Source : [Microsoft Learn](https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-process_mitigation_side_channel_isolation_policy).
- [DOC] `Enable-MMAgent -PageCombining` est un réglage **global à la machine**, qui exige les droits administrateur. Source : [Enable-MMAgent](https://learn.microsoft.com/en-us/powershell/module/mmagent/enable-mmagent). Sur la machine de dev, `Get-MMAgent` sans élévation renvoie « Accès refusé ». L'état par défaut de Windows 11 client n'a donc pas pu être lu ici, et je n'ai trouvé aucune source Microsoft qui l'affirme pour Windows 11.
- KSM : la doc noyau prévient que le scan peut consommer beaucoup de CPU ([docs.kernel.org KSM](https://docs.kernel.org/6.0/admin-guide/mm/ksm.html)). Je n'ai pas trouvé de taux d'économie publié et fiable pour des charges de type navigateur.

**Peut-on l'activer seulement pour les processus de Volt ?** Non. Il n'existe aucune API d'**opt-in** par processus. Il n'y a qu'un **opt-out** (`DisablePageCombine`). L'activation est globale, exige les droits admin et touche tous les processus de l'utilisateur, pas seulement Volt.

**Applicabilité à Volt** : nulle pour un produit grand public. Activer un réglage global de sécurité chez l'utilisateur est inacceptable. De plus, depuis 1903, les pages fusionnables se limitent presque aux pages nulles, qui ne correspondent pas à l'état utile d'un onglet.

**Risque** : élevé (canal auxiliaire démontré en recherche). **Gain attendu** : ≈ 0 [HYPOTHÈSE, vu les restrictions depuis 1903]. **Verdict : à écarter.**

---

## 2. Compression mémoire (magasin compressé de Windows, zram/zswap)

**Principe.** Au lieu d'écrire sur disque les pages peu utilisées, l'OS les compresse dans un magasin gardé en RAM. Les relire coûte une décompression, nettement moins chère qu'une lecture sur disque.

**Chiffres et faits publiés**
- [MESURÉ, chiffre annoncé par Microsoft] Windows 10 (build 10525, 2015) : les pages peu utilisées sont compressées « typiquement de 40 % » et gardées dans un magasin hébergé par le processus System (aujourd'hui « Memory Compression »). Seules les pages déjà compressées partent ensuite vers le disque. Source : billet Insider relayé par [Petri](https://petri.com/memory-compression-in-windows-10-threshold-2/) ; je n'ai pas retrouvé le billet original.
- [DOC] Algorithme : **Xpress** (décompressible par `RtlDecompressBufferEx`). Source : FireEye/Mandiant, [Finding Evil in Windows 10 Compressed Memory](https://cloud.google.com/blog/topics/threat-intelligence/finding-evil-in-windows-ten-compressed-memory-part-one) et [Black Hat USA 2019](https://i.blackhat.com/USA-19/Thursday/us-19-Sardar-Paging-All-Windows-Geeks-Finding-Evil-In-Windows-10-Compressed-Memory.pdf).
- [DOC] zram (Linux) : la doc noyau table sur un ratio de 2:1 ; algorithmes lzo, lz4 et zstd au choix. Source : [kernel.org zram](https://docs.kernel.org/6.14/admin-guide/blockdev/zram.html). Le wiki Arch cite un ratio courant de 1:3 avec zstd, ce qui est une source secondaire.
- **Ratio sur des pages de navigateur** : je n'ai trouvé **aucune mesure publiée**. Le tas JS, le DOM et les tuiles décodées se compressent sans doute bien, les données déjà compressées (JPEG, vidéo, WASM) très mal [HYPOTHÈSE].
- **Coût de décompression au réveil** : je n'ai trouvé aucune mesure publiée spécifique à Windows ou aux navigateurs.

**Comment faire entrer les processus gelés de Volt dans le magasin compressé**
- [DOC] `TrySuspend` « permet à l'OS de réutiliser la mémoire du processus de rendu ». Volt l'utilise déjà. Source : [ICoreWebView2_3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3).
- [DOC] `MemoryUsageTargetLevel=Low` « peut faire basculer une partie de la mémoire vers le disque ». Les scripts continuent de tourner. Il est **déconseillé de mélanger** cet appel avec `TrySuspend`/`Resume`, et le régler pendant une suspension est ignoré. Source : [ICoreWebView2_19](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19). Volt mélange les deux en Mode Jeu (Low sur les onglets inactifs, puis `TrySuspend` après 30 s). Ce n'est pas dangereux, mais c'est contraire à la recommandation.
- [DOC] `SetProcessInformation(ProcessMemoryPriority, MEMORY_PRIORITY_VERY_LOW)` est une indication donnée au gestionnaire mémoire pour le rognage du working set. Source : [MEMORY_PRIORITY_INFORMATION](https://learn.microsoft.com/windows/win32/api/processthreadsapi/ns-processthreadsapi-memory_priority_information). Le lien avec les 8 listes standby par priorité ne vient que d'une source communautaire (Bitsum, citant *Windows Internals*), pas de Microsoft.
- `EmptyWorkingSet` / `SetProcessWorkingSetSize(-1,-1)` : retire les pages du working set. Les pages privées modifiées passent par la liste « modified », d'où le gestionnaire de magasin peut les compresser [HYPOTHÈSE fondée sur le fonctionnement décrit ci-dessus, non vérifiée dans une doc Microsoft]. **Mise en garde** : sans pression mémoire, cela fait surtout baisser le *working set* affiché, une métrique cosmétique, au prix de fautes de page au réveil. Le *commit* ne baisse pas.
- `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` (EcoQoS) : agit sur le CPU (fréquence et cœurs efficaces), pas sur la mémoire. Microsoft ne publie aucun pourcentage d'économie. Sources : [Introducing EcoQoS](https://devblogs.microsoft.com/performance-diagnostics/introducing-ecoqos/), [PROCESS_POWER_THROTTLING_STATE](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/ns-processthreadsapi-process_power_throttling_state). Volt l'applique déjà quand la fenêtre est réduite.

**Point délicat : un processus de rendu peut servir plusieurs onglets.** Avec `--renderer-process-limit` ou le regroupement par site (§3.1), baisser la priorité mémoire d'un processus touche aussi les autres onglets qu'il héberge. Il faut d'abord savoir quels cadres vivent dans quel processus. WebView2 expose la liste des processus et des cadres qu'ils hébergent (`ICoreWebView2Environment8::GetProcessInfos`, et une variante « étendue » donnant les cadres par processus de rendu, à vérifier dans la référence du SDK utilisé). Il faut aussi vérifier qu'`OpenProcess(PROCESS_SET_INFORMATION)` réussit sur un processus de rendu sandboxé. `perf::background` ignore silencieusement les échecs, donc on ne sait pas aujourd'hui si l'appel aboutit [À VÉRIFIER].

**Risque** : faible (réglages réversibles). Le danger est un réveil lent si on rogne trop tôt. **Gain attendu** [HYPOTHÈSE] : sous pression mémoire seulement, quelques dizaines de Mo de RAM physique par processus gelé, mais aucun gain de commit. Sans pression mémoire, le gain est nul ou cosmétique.

---

## 3. Côté Chromium/V8

### 3.1 Process-per-site, site-per-process, limite de processus : le vrai levier « 50 × YouTube »

- [MESURÉ] Site Isolation (un processus par site) : **+9 à 13 % de mémoire totale**, moins de 2,25 % de latence de chargement en plus. Source : Reis et al., [USENIX Security 2019](https://www.usenix.org/conference/usenixsecurity19/presentation/reis). Désactiver Site Isolation récupérerait ce surcoût, mais rouvrirait Spectre entre sites. **À exclure.**
- [DOC] La limite de processus de Chromium est **« souple »**. Une fois la limite dépassée, Chromium « essaie de réutiliser au hasard des processus *du même site* ». La réutilisation respecte le verrouillage par site, donc la limite ne mélange pas des sites différents. Le mode *process-per-site* « regroupe toutes les instances d'un site dans un seul processus ». Source : [process_model_and_site_isolation.md](https://chromium.googlesource.com/chromium/src/+/main/docs/process_model_and_site_isolation.md).
  - Conséquence pour Volt : sur les machines ≤ 8 Go, `--renderer-process-limit` fait déjà regrouper les onglets YouTube entre eux. Sur une machine de 16 Go hors Mode Jeu, il n'y a pas de limite, donc environ un processus par onglet.
- [DOC, pas de chiffre] Expérience Chrome « Process sharing » : `--enable-features=ProcessPerSiteUpToMainFrameThreshold:ProcessPerSiteMainFrameThreshold/20` regroupe jusqu'à 20 documents principaux du même site dans un seul processus. Objectifs annoncés : moins de mémoire et un meilleur LCP. Le document de conception prévoit une légère régression de la réactivité (FID). Aucun résultat chiffré n'est publié. Source : [developer.chrome.com/blog/process-sharing-experiment](https://developer.chrome.com/blog/process-sharing-experiment).
- **Ce qui est réellement dédupliqué** dans un processus partagé [HYPOTHÈSE fondée sur l'architecture] : coût fixe du processus, isolate V8 et son cache de compilation en mémoire, ressources décodées du cache mémoire de Blink, polices, cache de styles. C'est exactement la logique « ne pas stocker 50 fois la même chose ».
- **Risques** :
  - un onglet planté ou figé emporte ses voisins du même site ;
  - un onglet YouTube lourd ralentit les autres (fil principal partagé) ;
  - les drapeaux de navigateur ne sont « pas destinés à la production » et peuvent disparaître à tout moment ([WebView2 browser flags](https://learn.microsoft.com/microsoft-edge/webview2/concepts/webview-features-flags)).
  - La sécurité entre sites n'est pas affaiblie, puisque le regroupement reste à l'intérieur d'un même site.

**Gain attendu** [HYPOTHÈSE à mesurer] : sur 50 onglets du même site sur une machine sans limite, c'est le plus gros gain possible en *commit*. Volt estime lui-même dans son code un coût fixe de 30 à 80 Mo par processus, mais ce chiffre n'est pas sourcé. Il faut le mesurer.

### 3.2 Pointer compression, builtins embarqués, snapshot V8 : déjà actifs, rien à faire

- [MESURÉ] Pointer compression : jusqu'à **−43 % de tas V8**, jusqu'à **−20 % de mémoire du processus de rendu** sur desktop. Source : [v8.dev/blog/pointer-compression](https://v8.dev/blog/pointer-compression). Elle est active dans Chrome et Edge, donc dans WebView2. Volt n'a aucun levier dessus.
- [MESURÉ] Builtins embarqués, partagés entre isolates et processus parce que mappés depuis le binaire : **−19 % de tas V8 médian**, **−4 % de processus de rendu médian**, soit 1,9 Mo économisés au 50e percentile. Source : [v8.dev/blog/embedded-builtins](https://v8.dev/blog/embedded-builtins). C'est la « keyframe partagée » au sens strict, et elle est déjà acquise.
- [DOC] Snapshot de démarrage : chaque isolate désérialise son tas initial depuis un snapshot commun. Un snapshot plus gros coûte de la mémoire à chaque isolate. Sources : [lazy deserialization](https://v8.dev/blog/lazy-deserialization), [custom startup snapshots](https://v8.dev/blog/custom-startup-snapshots). L'embarqueur WebView2 ne peut pas fournir son propre snapshot. Pas de levier.

### 3.3 Cache de code V8 (le « ne pas recalculer »)

- [DOC] Deux niveaux : un cache en mémoire par isolate, et un cache sérialisé sur disque qui **partage le code compilé entre processus et entre sessions**. Source : [v8.dev/blog/code-caching-for-devs](https://v8.dev/blog/code-caching-for-devs).
- [MESURÉ] Depuis Chrome 66, la mise en cache après l'exécution du code de premier niveau donne **−20 à −40 % de temps de parsing et de compilation** au chargement initial. Source : [v8.dev/blog/improved-code-caching](https://v8.dev/blog/improved-code-caching).
- **Applicabilité** : c'est automatique dans WebView2, à condition que le dossier de données utilisateur (UDF) persiste et ne soit pas vidé. C'est ce qui rend bon marché un rechargement après déchargement (§4). **Action** : ne jamais vider le cache HTTP et le cache de code à la fermeture ou au déchargement d'un onglet.

### 3.4 Memory Saver, tab discarding, Sleeping Tabs (chiffres publiés)

- [MESURÉ, Google] Memory Saver + Energy Saver (Chrome 108) : « jusqu'à 40 % et 10 Go de mémoire en moins ». Source : [blog.google](https://blog.google/products/chrome/new-chrome-features-to-save-battery-and-make-browsing-smoother/).
- [MESURÉ, Microsoft] Sleeping Tabs : en moyenne −32 % de mémoire, et un onglet endormi consomme 37 % de CPU en moins. Source : [Edge blog, déc. 2020](https://blogs.windows.com/msedgedev/2020/12/09/sleeping-tabs-beta-performance/). Edge 100 : « chaque onglet endormi économise en moyenne **85 % de mémoire et 99 % de CPU** », et 8 % d'onglets en plus peuvent dormir (pages qui partagent une instance de navigation). Source : [Edge blog, avr. 2022](https://blogs.windows.com/msedgedev/2022/04/07/sleeping-tabs-edge-100-improvements/). Microsoft ne décrit pas la méthode de mesure.
- [DOC] `TrySuspend` est « similaire à mettre un onglet en veille dans Edge » : timers et animations en pause, CPU minimal. Il exige `IsVisible=false`. Le son, entre autres conditions, empêche la suspension. Source : [ICoreWebView2_3](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_3).
- [DOC] Discarding : un onglet déchargé libère tout et se recharge au clic. C'est sans gêne pour une page de contenu, gênant pour une page interactive. Source : [Memory and Energy Saver modes](https://developer.chrome.com/blog/memory-and-energy-saver-mode).

Volt fait déjà gel puis déchargement. Le gain restant est dans les **seuils** et dans la **qualité du réveil** (le delta, §4).

### 3.5 BFCache

- [DOC] Snapshot complet de la page en mémoire, tas JS compris. Retour arrière ou avant = environ 1 navigation sur 10 sur desktop. Source : [web.dev/articles/bfcache](https://web.dev/articles/bfcache).
- **Applicabilité** : automatique dans WebView2. C'est une keyframe *coûteuse* en RAM. Volt n'a pas d'API pour la purger ou la limiter. `MemoryUsageTargetLevel=Low` peut « vider des données en cache » ([performance WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance)), mais on ne sait pas si cela inclut le BFCache [À VÉRIFIER].

### 3.6 PartitionAlloc reclaim

- [MESURÉ, une seule mesure locale] Une purge ponctuelle au passage en arrière-plan d'un processus de rendu a libéré environ 400 Kio sur un site d'actualité (Linux). Chiffre tiré d'un message de commit Chromium vu en résultat de recherche (lien direct non vérifié) ; mécanisme décrit dans [PartitionAlloc.md](https://chromium.googlesource.com/chromium/src/+/HEAD/base/allocator/partition_allocator/PartitionAlloc.md). Le mécanisme est interne, Volt n'a pas de levier. C'est faible.

### 3.7 Freezing et throttling des timers

- [MESURÉ, Google] Budget de timers en arrière-plan (Chrome 57) : environ 1 % d'un cœur au maximum pour un onglet trop gourmand, **25 % d'onglets d'arrière-plan « occupés » en moins**. Les onglets avec son, WebSocket ou WebRTC sont exemptés. Source : [blog.chromium.org, mars 2017](https://blog.chromium.org/2017/03/reducing-power-consumption-for.html).
- [MESURÉ, Google] Intensive wake-up throttling (Chrome 88) : au plus un réveil de timer par minute après 5 min cachée. Le document de conception cite jusqu'à 5 fois moins de CPU et jusqu'à 1,25 h d'autonomie en plus. Source : [developer.chrome.com/blog/timer-throttling-in-chrome-88](https://developer.chrome.com/blog/timer-throttling-in-chrome-88). Une variante à 10 s pour les pages déjà chargées a été proposée ([Intent to Ship](https://groups.google.com/a/chromium.org/g/blink-dev/c/5SZB2CFFGqE)). Son état actuel n'est pas vérifié.
- [DOC] Freezing on Energy Saver (Chrome 133) : gèle les onglets d'arrière-plan gourmands en CPU, seulement quand Energy Saver est actif. Source : [developer.chrome.com/blog/freezing-on-energy-saver](https://developer.chrome.com/blog/freezing-on-energy-saver).
- **Applicabilité** : tout cela se déclenche dès que la page est *cachée*, c'est-à-dire avec `IsVisible=false`, ce que Volt fait déjà. Le gel de type Energy Saver dépend d'un réglage d'Edge, pas d'une API WebView2. `TrySuspend` en est l'équivalent explicite.

### 3.8 Drapeaux et options réellement utilisables dans WebView2

| Levier | Support | Remarque |
|---|---|---|
| `TrySuspend` / `Resume` | API stable | Déjà utilisé |
| `MemoryUsageTargetLevel` | API stable (SDK 1.0.1823.32 et ultérieurs) | Fait pour les onglets qui *doivent* continuer à tourner (son) |
| `IsVisible` | API stable | Déclenche tout le throttling Chromium |
| Environnement partagé | API stable | Recommandé par Microsoft ([performance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance)) |
| `--renderer-process-limit`, `--process-per-site`, `--enable-features=ProcessPerSiteUpToMainFrameThreshold` | Drapeau Chromium, **non supporté en production** | Peut disparaître ; à revérifier à chaque runtime |
| `--js-flags=--scavenger_max_new_space_capacity_mb=8` | Donné en exemple dans la doc des drapeaux WebView2 | Plafonne la jeune génération par isolate ; aucun chiffre publié |
| `OpenTaskManagerWindow` (ICoreWebView2_6) | API stable | Gestionnaire des tâches du moteur, utile pour mesurer |

---

## 4. Image clé + delta : état de l'art et ce qui est réaliste

- **Snapshot de processus (CRIU)** : propre à Linux. Les études sur Docker montrent un temps de capture proportionnel à la mémoire ([arXiv 2307.12113](https://arxiv.org/abs/2307.12113)). Les applications GPU ne passent pas toujours ([CHEP 2024](https://www.epj-conferences.org/articles/epjconf/abs/2024/05/epjconf_chep2024_07046/epjconf_chep2024_07046.html)). Je n'ai trouvé aucune mesure publiée sur Chromium. Windows n'a pas d'équivalent public, et un processus de rendu Chromium sandboxé, lié au processus GPU et au processus navigateur par IPC, ne se restaure pas indépendamment. **Irréaliste pour Volt.**
- **Sérialisation de l'état d'un onglet Chromium** : un projet étudiant (Harvey Mudd Clinic 2017) l'a exploré, sans chiffres disponibles ([hmc.edu](https://www.hmc.edu/clinic/2017/serializing-chromium-tab-state/)). Ce n'est pas exposé par WebView2.
- **Keyframes qui existent vraiment** : le snapshot V8 et les builtins (communs à tous, §3.2), le BFCache (complet, par page), et pour un onglet déchargé, **le cache HTTP et le cache de code V8 sur disque, partagés par tous les onglets**.
- **Le delta réaliste pour Volt** : quand Volt décharge un onglet (`wv = None`), il ne garde aujourd'hui que l'URL. Un delta léger, de quelques Ko par onglet [HYPOTHÈSE sur la taille], peut être capturé par `ExecuteScript` juste avant le déchargement :
  - position de défilement ;
  - champs de formulaire non sensibles ;
  - pour une `<video>`, `currentTime` (pour YouTube, une URL avec `&t=<s>` suffit).
  Le réveil devient alors : keyframe (cache HTTP + cache de code, rechargement « hot ») + delta réappliqué après `NavigationCompleted`.
  - Limite : l'historique arrière et avant de l'onglet est perdu, et WebView2 n'a pas d'API pour le restaurer.
  - Limite : l'état JS en mémoire (application monopage) est perdu.
  - Ne jamais capturer les mots de passe ni les champs `type=password`.

**Gain** : aucun gain mémoire de plus que le déchargement actuel, mais un réveil bien plus acceptable, ce qui permet de **décharger plus tôt** (seuil plus bas que 20 min). C'est là que se situe le gain mémoire indirect.

---

## 5. Niveaux de qualité pour les onglets cachés ou peu visibles

- [DOC] Piste vidéo désactivée pour la lecture MSE (le cas de YouTube) dans un onglet non visible, à partir de Chrome 62, pour les vidéos dont l'intervalle entre images clés est inférieur à 5 s. Les vidéos sans son sont mises en pause en arrière-plan (desktop à partir de Chrome 63). Sources : [Media updates in Chrome 61](https://developer.chrome.com/blog/media-updates-in-chrome-61), [Chrome 63/64](https://developer.chrome.com/blog/media-updates-in-chrome-63-64/). C'est exactement « 50 YouTube qui ne décodent pas 50 vidéos ». **C'est automatique, à condition que l'onglet soit réellement caché** (`IsVisible=false`).
- [DOC, spécification HTML] `requestAnimationFrame` n'est pas appelé dans un document caché, donc aucune image n'est produite.
- Tuiles GPU : le compositeur fixe son budget de tuiles selon la visibilité et la pression mémoire. Je n'ai pas trouvé de source primaire à jour confirmant que les tuiles d'un onglet caché sont *libérées* et pas seulement dépriorisées ([how_cc_works.md](https://chromium.googlesource.com/chromium/src/+/main/docs/how_cc_works.md) ; le détail des politiques mémoire vient de sources secondaires) [À VÉRIFIER].
- Occlusion native Windows : si la fenêtre est recouverte, Chromium réduit la fréquence d'images ([commit Chromium](https://chromium.googlesource.com/chromium/src/+/55bef1db2cd4e5da5ef1636ff3025f1180f61399)). C'est automatique.
- Images décodées à plus basse résolution pour les onglets peu visibles : **je n'ai trouvé aucun mécanisme documenté ni contrôlable** depuis WebView2.
- Qualité vidéo réduite pour un onglet en arrière-plan qui joue du son (YouTube en musique de fond) : possible seulement par un script propre au site, fragile et non recommandé. La piste vidéo est de toute façon coupée par Chromium si l'onglet est caché.
- Contrôlable par Volt : `IsVisible`, `MemoryUsageTargetLevel`, EcoQoS et priorité du processus (déjà fait quand la fenêtre est réduite). Le reste est décidé par le moteur.

---

## 6. Top 5 des actions pour Volt (rapport gain/effort)

Protocole commun à toutes les mesures :
- **Matériel et environnement** : même machine, secteur branché, runtime WebView2 figé (noter la version, ici 157.0.4341.60), redémarrage de Volt entre chaque passe, UDF déjà chauffé par une passe à blanc.
- **Alternance** : A B A B A B (3 passes ou plus par bras), puis médiane et écart interquartile. Utiliser `VOLT_ARGS` et `VOLT_FREEZE`, qui existent déjà, pour changer de bras sans recompiler.
- **Scénario « 50 YouTube »** : 50 URL `youtube.com/watch?v=…` distinctes, muettes, ouvertes en 5 min. Un seul onglet actif. Mesures à T+2 min, T+10 min et T+25 min (après déchargement).
- **Métriques**, sur l'arbre de processus `msedgewebview2.exe` issu de `BrowserProcessId`, plus Volt lui-même :
  - **Commit** : somme de `PrivateUsage` via `GetProcessMemoryInfo` / `PROCESS_MEMORY_COUNTERS_EX`. C'est la métrique principale.
  - **Working set** : somme de `WorkingSetSize`, plus le working set du processus « Memory Compression » (à relever avec élévation) pour voir ce qui a été déplacé et non économisé.
  - **Mémoire système** : `\Memory\Committed Bytes` et `\Memory\Available MBytes` (`Get-Counter`).
  - **CPU** : somme des deltas de `GetProcessTimes` (noyau + utilisateur) sur une fenêtre de 10 min onglets cachés, en secondes CPU par minute.
  - **Temps de réveil** : de l'activation de l'onglet jusqu'au premier `requestAnimationFrame` reçu par `postMessage` (script injecté), et jusqu'au retour d'`ExecuteScript("1")`. Plus le delta de `PageFaultCount` du processus de rendu pendant le réveil.

### 1. Regrouper les onglets du même site sur toutes les machines
Bras A : réglage actuel (pas de limite au-delà de 8 Go). Bras B : `VOLT_ARGS="--enable-features=ProcessPerSiteUpToMainFrameThreshold:ProcessPerSiteMainFrameThreshold/20"` (et B' : `--process-per-site`). Attention : `--enable-features` passé dans `VOLT_ARGS` doit être fusionné avec celui que Volt génère déjà, sinon l'un écrase l'autre [À VÉRIFIER sur le comportement de wry et de Chromium avec un drapeau en double].
- **Métrique principale** : commit total à T+2 min. Contrôler aussi le nombre de processus de rendu.
- **Garde-fou** : latence d'entrée sur l'onglet actif pendant qu'un autre onglet YouTube du même processus charge (INP via DevTools ou `PerformanceObserver` injecté).
- **Gain/effort** : effort minime (un drapeau), gain potentiellement le plus élevé sur le cas « N onglets d'un même site ». Ce gain n'est pas chiffré publiquement [HYPOTHÈSE].
- **Risque** : drapeau non supporté en production, plantage partagé entre onglets du même site.

### 2. Abaisser les seuils de déchargement, avec un « delta » de réveil
Bras A : déchargement à 20 min. Bras B : déchargement à 5 min, plus capture et restauration du défilement et de `currentTime` (YouTube via `&t=`).
- **Métriques** : commit à T+10 min, et temps de réveil d'un onglet déchargé (de la navigation jusqu'au premier rAF, puis jusqu'à la position restaurée). Comparer ce temps à celui d'un onglet seulement gelé.
- **Référence publiée** : Edge annonce −85 % de mémoire par onglet endormi ([Edge 100](https://blogs.windows.com/msedgedev/2022/04/07/sleeping-tabs-edge-100-improvements/)), et un onglet déchargé libère en principe tout son processus.
- **Gain/effort** : élevé pour un effort moyen (environ 30 lignes JS et Rust).
- **Risque** : perte de l'état des applications monopages et de l'historique de l'onglet. Exclure les onglets avec formulaire modifié. Ne jamais capturer de mot de passe.

### 3. `MemoryUsageTargetLevel=Low` pour les onglets cachés qui jouent du son, et ne plus le mélanger avec `TrySuspend`
Aujourd'hui, les onglets avec son ne sont jamais gelés et restent au niveau Normal hors Mode Jeu. C'est précisément le cas d'usage documenté de Low ([ICoreWebView2_19](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2_19)). En Mode Jeu, retirer Low des onglets qui seront ensuite suspendus, conformément à la doc.
- **Mesure** : 5 onglets YouTube cachés qui jouent du son ; bras A au niveau Normal, bras B au niveau Low. Relever le commit, le working set et le CPU sur 10 min, et compter les coupures audio (écoute plus événements `waiting` injectés).
- **Gain/effort** : effort très faible. Gain [HYPOTHÈSE] modéré, sans chiffre publié par Microsoft.
- **Risque** : à-coups audio si des pages ressortent du disque ; la doc prévient que les performances « peuvent être affectées ».

### 4. Priorité mémoire `VERY_LOW` pour les processus de rendu dont *tous* les cadres sont suspendus
Relier les processus aux onglets via les API d'information de processus de WebView2 (§2). Ne toucher qu'un processus dont tous les onglets sont gelés. Remettre `NORMAL` avant `Resume`. Vérifier d'abord que `OpenProcess(PROCESS_SET_INFORMATION)` réussit sur un processus de rendu.
- **Mesure** : sous pression mémoire contrôlée (un programme de test qui alloue et touche RAM − 1 Go), avec 30 onglets gelés. Bras A sans priorité, bras B avec `VERY_LOW`. Relever le working set de l'onglet *actif* (il doit moins souffrir), sa latence d'entrée, et le temps de réveil des onglets gelés.
- **Gain/effort** : moyen pour un effort moyen. Le gain n'existe que sous pression mémoire, ce qui est justement le cas des machines de 8 Go [HYPOTHÈSE].
- **Risque** : faible. Réveil plus lent des onglets gelés, par conception.
- **À ne pas faire** : un `EmptyWorkingSet` systématique. Il fait baisser le working set affiché sans baisser le commit, et ajoute des fautes de page au réveil. Si on veut le tester, en faire un bras C distinct.

### 5. Plafonner la jeune génération V8 par isolate (bras exploratoire)
Bras B : `VOLT_ARGS="--js-flags=--scavenger_max_new_space_capacity_mb=8"`. Le drapeau est cité en exemple par la [doc WebView2](https://learn.microsoft.com/microsoft-edge/webview2/concepts/webview-features-flags).
- **Mesures** : commit à T+2 min sur le scénario 50 YouTube, CPU de l'onglet actif pendant la lecture d'une vidéo (le ramasse-miettes peut tourner plus souvent), et INP.
- **Gain/effort** : effort nul. Le gain dépend du nombre d'isolates, et donc se cumule avec l'action 1 ou s'annule avec elle [HYPOTHÈSE].
- **Risque** : plus de CPU passé dans le ramasse-miettes, et drapeau non supporté en production.

### Écarté, avec raison
- **Page Combining** : réglage global, admin requis, canal auxiliaire démontré, quasi inutile depuis 1903 (§1).
- **Désactiver Site Isolation** : environ −9 à 13 % de mémoire, au prix de l'isolation contre Spectre (§3.1).
- **Snapshot et restauration de processus façon CRIU** : impossible sous Windows avec WebView2 (§4).
- **Baisser la qualité d'image ou de décodage** : rien de contrôlable. Ce qui compte (piste vidéo coupée, plus de rAF, throttling) est déjà automatique quand `IsVisible=false`.

---

## Limites de cette recherche

- Je n'ai trouvé aucune mesure publiée pour : le ratio de compression des pages de navigateur sous Windows, le coût de décompression au réveil, le coût CPU de Page Combining, le gain mémoire de `MemoryUsageTargetLevel=Low`, ni pour l'expérience « Process sharing ».
- L'état par défaut de Page Combining sous Windows 11 n'a pas été confirmé, faute d'une source Microsoft et faute de droits admin pour lancer `Get-MMAgent` sur la machine de dev.
- Les chiffres de Google et de Microsoft sont des valeurs annoncées « jusqu'à » ou des moyennes, sans méthode détaillée. Ils indiquent un ordre de grandeur, pas un gain garanti pour Volt.
