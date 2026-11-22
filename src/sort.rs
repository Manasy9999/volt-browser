//! Rangement automatique des favoris sans dossier.
//!
//! Tout est local et déterministe : on classe d'abord par domaine connu (l'entrée la plus
//! précise l'emporte), puis à défaut par mots de l'hôte, puis du titre. Dans le doute on ne
//! classe pas : un favori laissé sans dossier vaut mieux qu'un mauvais rangement.

use std::collections::HashSet;

/// Un dossier proposé et les favoris sans dossier qu'on y rangerait.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub folder: String,     // nom du dossier : celui de l'utilisateur s'il en existe un équivalent, sinon le nom de la catégorie
    pub new: bool,          // true = dossier à créer
    pub icon: &'static str, // emoji de la catégorie (pour un dossier à créer)
    pub color: &'static str, // couleur "#rrggbb" de la catégorie (pour un dossier à créer)
    pub urls: Vec<String>,  // favoris proposés, dans l'ordre reçu
}

struct Category {
    name: &'static str,
    icon: &'static str,
    color: &'static str,
    synonyms: &'static [&'static str], // noms de dossiers équivalents (casse, accents et pluriel ignorés)
    domains: &'static [&'static str],
    host_words: &'static [&'static str], // fragment exact de l'hôte (découpé sur « . » et « - ») ; « mot* » = début de fragment
    title_words: &'static [&'static str], // mots entiers du titre, en minuscules et sans accents
}

// ---------------------------------------------------------------------------------------------
// Tables de domaines : une constante par catégorie, triée, sans doublon entre catégories
// (vérifié par les tests). Un domaine couvre aussi ses sous-domaines.
// ---------------------------------------------------------------------------------------------

const JEUX: &[&str] = &[
    "activision.com", "ankama.com", "battle.net", "blizzard.com", "bungie.net", "callofduty.com",
    "chess.com", "crazygames.com", "curseforge.com", "dexerto.com", "dofus.com", "ea.com",
    "eneba.com", "epicgames.com", "faceit.com", "fextralife.com", "fortnite.com", "g2a.com",
    "gameblog.fr", "gamekult.com", "gamergen.com", "gamesplanet.com", "gamespot.com",
    "gaming.amazon.com", "geoguessr.com", "gog.com", "greenmangaming.com", "gx.games",
    "howlongtobeat.com", "hoyolab.com", "hoyoverse.com", "humblebundle.com", "igdb.com", "ign.com",
    "instant-gaming.com", "itch.io", "jeuxactu.com", "jeuxvideo.com", "kinguin.net",
    "kongregate.com", "leagueoflegends.com", "lichess.org", "maxroll.gg", "micromania.fr",
    "millenium.org", "minecraft.net", "miniclip.com", "mobalytics.gg", "modrinth.com",
    "nexusmods.com", "nintendo.com", "nintendo.fr", "op.gg", "overwolf.com", "pathofexile.com",
    "pcgamer.com", "playstation.com", "playvalorant.com", "pokemon.com", "poki.com", "polygon.com",
    "rawg.io", "riotgames.com", "roblox.com", "rockstargames.com", "runescape.com", "speedrun.com",
    "steamcommunity.com", "steamdb.info", "steampowered.com", "store.steampowered.com",
    "tracker.gg", "u.gg", "ubisoft.com", "wowhead.com", "xbox.com", "y8.com",
];

const VIDEOS: &[&str] = &[
    "6play.fr", "allocine.fr", "animationdigitalnetwork.com", "arte.tv", "betaseries.com",
    "bilibili.com", "canalplus.com", "crunchyroll.com", "dailymotion.com", "disneyplus.com",
    "ecranlarge.com", "france.tv", "hbomax.com", "hulu.com", "imdb.com", "justwatch.com",
    "kick.com", "letterboxd.com", "m6.fr", "m6plus.fr", "max.com", "molotov.tv", "mubi.com",
    "nebula.tv", "netflix.com", "nicovideo.jp", "ocs.fr", "odysee.com", "paramountplus.com",
    "plex.tv", "pluto.tv", "premiere.fr", "primevideo.com", "programme-tv.net", "rakuten.tv",
    "rottentomatoes.com", "rumble.com", "senscritique.com", "streamable.com", "telerama.fr",
    "tf1.fr", "themoviedb.org", "trakt.tv", "tubi.tv", "tv.apple.com", "twitch.tv", "viki.com",
    "vimeo.com", "youtu.be", "youtube-nocookie.com", "youtube.com",
];

const MUSIQUE: &[&str] = &[
    "audiomack.com", "bandcamp.com", "beatport.com", "cheriefm.fr", "deezer.com", "discogs.com",
    "fip.fr", "funradio.fr", "genius.com", "hypem.com", "jamendo.com", "last.fm", "mixcloud.com",
    "music.amazon.com", "music.amazon.fr", "music.apple.com", "music.youtube.com",
    "musicbrainz.org", "musixmatch.com", "nostalgie.fr", "nrj.fr", "pandora.com", "pitchfork.com",
    "qobuz.com", "radio.fr", "radio.net", "radiofrance.fr", "rateyourmusic.com", "rfm.fr",
    "setlist.fm", "shazam.com", "skyrock.com", "songkick.com", "songsterr.com", "soundcloud.com",
    "splice.com", "spotify.com", "suno.com", "tidal.com", "tunein.com", "udio.com",
    "ultimate-guitar.com",
];

const RESEAUX: &[&str] = &[
    "9gag.com", "artstation.com", "behance.net", "bereal.com", "bsky.app", "buymeacoffee.com",
    "deviantart.com", "discord.com", "discord.gg", "discordapp.com", "dribbble.com", "facebook.com",
    "fb.com", "flickr.com", "guilded.gg", "imgur.com", "instagram.com", "ko-fi.com", "lemmy.world",
    "linkedin.com", "linktr.ee", "mastodon.social", "medium.com", "meetup.com", "nextdoor.com",
    "patreon.com", "pinterest.com", "pinterest.fr", "quora.com", "redd.it", "reddit.com",
    "snapchat.com", "substack.com", "threads.net", "tiktok.com", "tipeee.com", "tumblr.com",
    "twitter.com", "vk.com", "weibo.com", "x.com",
];

const DEV: &[&str] = &[
    "angular.dev", "aws.amazon.com", "azure.microsoft.com", "bitbucket.org", "bolt.new",
    "cloud.google.com", "cloudflare.com", "code.visualstudio.com", "codepen.io", "codesandbox.io",
    "codewars.com", "colab.research.google.com", "console.anthropic.com", "cppreference.com",
    "crates.io", "cursor.com", "dev.to", "devdocs.io", "developer.mozilla.org", "developpez.com",
    "digitalocean.com", "docker.com", "docs.rs", "firebase.google.com", "fly.io",
    "freecodecamp.org", "github.com", "gitlab.com", "go.dev", "godbolt.org", "godotengine.org",
    "hackerrank.com", "hashnode.com", "heroku.com", "huggingface.co", "jetbrains.com",
    "jsfiddle.net", "kaggle.com", "kotlinlang.org", "kubernetes.io", "learn.microsoft.com",
    "leetcode.com", "localhost", "lovable.dev", "netlify.com", "nextjs.org", "nodejs.org",
    "npmjs.com", "ollama.com", "openrouter.ai", "ovh.com", "ovhcloud.com", "platform.openai.com",
    "portal.azure.com", "postman.com", "pypi.org", "python.org", "railway.app", "react.dev",
    "readthedocs.io", "readthedocs.org", "regex101.com", "render.com", "replicate.com",
    "replit.com", "rust-lang.org", "sourceforge.net", "stackblitz.com", "stackexchange.com",
    "stackoverflow.com", "supabase.com", "svelte.dev", "tailwindcss.com", "typescriptlang.org",
    "unity.com", "unrealengine.com", "v0.dev", "vercel.com", "vuejs.org", "w3schools.com",
];

const ACHATS: &[&str] = &[
    "action.com", "adidas.fr", "aliexpress.com", "amazon.co.uk", "amazon.com", "amazon.de",
    "amazon.es", "amazon.fr", "amazon.it", "asos.com", "auchan.fr", "backmarket.com",
    "backmarket.fr", "bestbuy.com", "boulanger.com", "but.fr", "carrefour.fr", "castorama.fr",
    "cdiscount.com", "conforama.fr", "courir.com", "cultura.com", "darty.com", "dealabs.com",
    "decathlon.fr", "deliveroo.fr", "e.leclerc", "ebay.com", "ebay.fr", "etsy.com", "fnac.com",
    "hm.com", "idealo.fr", "ikea.com", "intermarche.com", "jdsports.fr", "just-eat.fr", "kiabi.com",
    "laredoute.fr", "ldlc.com", "leboncoin.fr", "ledenicheur.fr", "lego.com", "leroymerlin.fr",
    "lidl.fr", "maisonsdumonde.com", "manomano.fr", "materiel.net", "newegg.com", "nike.com",
    "nocibe.fr", "rakuten.fr", "rueducommerce.fr", "sephora.fr", "shein.com", "shopify.com",
    "showroomprive.com", "snipes.com", "stockx.com", "temu.com", "topachat.com", "ubereats.com",
    "uniqlo.com", "veepee.fr", "vinted.com", "vinted.fr", "walmart.com", "wish.com", "zalando.fr",
    "zara.com",
];

const ACTUALITES: &[&str] = &[
    "01net.com", "20minutes.fr", "7sur7.be", "actu.fr", "apnews.com", "arstechnica.com",
    "bbc.co.uk", "bbc.com", "bfmtv.com", "bloomberg.com", "brut.media", "challenges.fr",
    "clubic.com", "cnn.com", "courrierinternational.com", "dna.fr", "euronews.com", "forbes.com",
    "france24.com", "franceinter.fr", "francetvinfo.fr", "frandroid.com", "ft.com",
    "huffingtonpost.fr", "huffpost.com", "humanite.fr", "journaldugeek.com", "konbini.com",
    "korben.info", "ladepeche.fr", "lapresse.ca", "latribune.fr", "lavoixdunord.fr", "lci.fr",
    "ledauphine.com", "lefigaro.fr", "lemonde.fr", "leparisien.fr", "lepoint.fr", "leprogres.fr",
    "lesechos.fr", "lesnumeriques.com", "lesoir.be", "letelegramme.fr", "lexpress.fr",
    "liberation.fr", "marianne.net", "mediapart.fr", "midilibre.fr", "msn.com", "news.google.com",
    "news.yahoo.com", "news.ycombinator.com", "next.ink", "nicematin.com", "nouvelobs.com",
    "numerama.com", "nytimes.com", "ouest-france.fr", "parismatch.com", "phonandroid.com",
    "presse-citron.net", "radio-canada.ca", "reuters.com", "rfi.fr", "rtbf.be", "rts.ch",
    "slate.fr", "sudouest.fr", "techcrunch.com", "tf1info.fr", "theguardian.com", "theverge.com",
    "tomsguide.fr", "washingtonpost.com", "wired.com", "wsj.com", "zdnet.fr",
];

const OUTILS: &[&str] = &[
    "1fichier.com", "1password.com", "accuweather.com", "airtable.com", "aistudio.google.com",
    "anthropic.com", "bitwarden.com", "calendar.google.com", "calendly.com", "canva.com",
    "capcut.com", "character.ai", "chatgpt.com", "civitai.com", "claude.ai", "cloudconvert.com",
    "convertio.co", "copilot.microsoft.com", "deepl.com", "deepseek.com", "docs.google.com",
    "doodle.com", "drive.google.com", "dropbox.com", "elevenlabs.io", "evernote.com", "figma.com",
    "gamma.app", "gemini.google.com", "gmail.com", "grok.com", "heygen.com", "higgsfield.ai",
    "icloud.com", "ideogram.ai", "ilovepdf.com", "keep.google.com", "klingai.com", "krea.ai",
    "laposte.net", "lastpass.com", "leonardo.ai", "live.com", "mail.google.com", "mail.yahoo.com",
    "mediafire.com", "meet.google.com", "mega.nz", "messenger.com", "meteociel.fr",
    "meteofrance.com", "microsoft365.com", "midjourney.com", "miro.com", "mistral.ai",
    "notebooklm.google.com", "notion.com", "notion.so", "obsproject.com", "office.com",
    "onedrive.com", "openai.com", "orange.fr", "outlook.com", "perplexity.ai", "photopea.com",
    "photos.google.com", "poe.com", "proton.me", "protonmail.com", "runwayml.com", "signal.org",
    "slack.com", "smallpdf.com", "t.me", "teams.microsoft.com", "telegram.org", "todoist.com",
    "translate.google.com", "trello.com", "virustotal.com", "wetransfer.com", "whatsapp.com",
    "windy.com", "zoom.us",
];

// « gouv.fr » attrape tous les sites de l'État ; une entrée plus précise ailleurs l'emporte.
const BANQUE_ADMIN: &[&str] = &[
    "agirc-arrco.fr", "alan.com", "allianz.fr", "ameli.fr", "axa.fr", "banquepopulaire.fr",
    "bforbank.com", "binance.com", "bnpparibas", "boursobank.com", "boursorama.com",
    "bouyguestelecom.fr", "bpifrance.fr", "caf.fr", "caisse-epargne.fr", "cic.fr", "coinbase.com",
    "credit-agricole.fr", "creditmutuel.fr", "doctolib.fr", "edf.fr", "enedis.fr", "engie.fr",
    "fortuneo.fr", "francetravail.fr", "gouv.fr", "grdf.fr", "groupama.fr", "hellobank.fr",
    "impots.gouv.fr", "info-retraite.fr", "kraken.com", "labanquepostale.fr", "laposte.fr",
    "lassuranceretraite.fr", "lcl.fr", "lydia-app.com", "macif.fr", "maif.fr", "matmut.fr",
    "mgen.fr", "monabanq.com", "msa.fr", "n26.com", "net-entreprises.fr", "paypal.com",
    "pole-emploi.fr", "qonto.com", "revolut.com", "service-public.fr", "sfr.fr", "sg.fr",
    "societegenerale.fr", "sosh.fr", "totalenergies.fr", "urssaf.fr", "wise.com",
];

const SAVOIR: &[&str] = &[
    "750g.com", "academia.edu", "allrecipes.com", "ankiweb.net", "archive.org", "arxiv.org",
    "babbel.com", "babelio.com", "brilliant.org", "britannica.com", "cairn.info", "cambridge.org",
    "cnrtl.fr", "codecademy.com", "coursera.org", "cuisineactuelle.fr", "cuisineaz.com",
    "desmos.com", "dictionary.com", "domestika.org", "duolingo.com", "ecoledirecte.com", "edx.org",
    "fun-mooc.fr", "futura-sciences.com", "geogebra.org", "goodreads.com", "gutenberg.org",
    "hal.science", "ina.fr", "index-education.net", "jstor.org", "kartable.fr", "khanacademy.org",
    "larousse.fr", "leconjugueur.lefigaro.fr", "lerobert.com", "linguee.com", "linguee.fr",
    "lumni.fr", "marmiton.org", "maxicours.com", "memrise.com", "merriam-webster.com",
    "nationalgeographic.com", "nature.com", "nih.gov", "openclassrooms.com", "persee.fr",
    "ptitchef.com", "quizlet.com", "researchgate.net", "reverso.net", "scholar.google.com",
    "sciencedirect.com", "sciencesetavenir.fr", "skillshare.com", "ted.com", "udemy.com",
    "universalis.fr", "vikidia.org", "wikihow.com", "wikimedia.org", "wikipedia.org",
    "wikiwand.com", "wiktionary.org", "wolframalpha.com", "wordreference.com",
];

const VOYAGES: &[&str] = &[
    "abritel.fr", "accor.com", "airbnb.com", "airbnb.fr", "airfrance.com", "airfrance.fr",
    "bison-fute.gouv.fr", "blablacar.com", "blablacar.fr", "bolt.eu", "bonjour-ratp.fr",
    "booking.com", "citymapper.com", "clubmed.fr", "easyjet.com", "edreams.fr", "emirates.com",
    "europcar.fr", "eurostar.com", "expedia.com", "expedia.fr", "flightradar24.com", "flixbus.com",
    "flixbus.fr", "getaround.com", "getyourguide.com", "gites-de-france.com", "heetch.com",
    "hostelworld.com", "hotels.com", "iledefrance-mobilites.fr", "kayak.com", "kayak.fr", "klm.fr",
    "lastminute.com", "lonelyplanet.com", "lufthansa.com", "mappy.com", "maps.google.com",
    "openstreetmap.org", "opodo.fr", "ouigo.com", "ratp.fr", "rentalcars.com", "rome2rio.com",
    "routard.com", "ryanair.com", "sixt.fr", "skyscanner.com", "skyscanner.fr", "sncf-connect.com",
    "sncf.com", "trainline.com", "trainline.fr", "transavia.com", "tripadvisor.com",
    "tripadvisor.fr", "trivago.com", "trivago.fr", "uber.com", "viamichelin.fr", "voyage-prive.com",
    "vrbo.com", "vueling.com", "waze.com", "wizzair.com",
];

const SPORT: &[&str] = &[
    "atptour.com", "basic-fit.com", "beinsports.com", "betclic.fr", "connect.garmin.com",
    "dazn.com", "espn.com", "eurosport.com", "eurosport.fr", "fbref.com", "fff.fr", "fifa.com",
    "flashscore.com", "flashscore.fr", "footmercato.net", "formula1.com", "fotmob.com", "goal.com",
    "komoot.com", "laliga.com", "lequipe.fr", "letour.fr", "ligue1.com", "livescore.com", "lnr.fr",
    "maxifoot.fr", "mlb.com", "motogp.com", "myfitnesspal.com", "nba.com", "nfl.com", "nhl.com",
    "ol.fr", "olympics.com", "om.fr", "onefootball.com", "parionssport.fdj.fr", "pmu.fr",
    "premierleague.com", "procyclingstats.com", "psg.fr", "rmcsport.bfmtv.com", "rolandgarros.com",
    "rugbyrama.fr", "sofascore.com", "sofoot.com", "strava.com", "transfermarkt.com",
    "transfermarkt.fr", "uefa.com", "ufc.com", "unibet.fr", "whoscored.com", "winamax.fr",
    "worldrugby.org", "wtatennis.com", "zwift.com",
];

// Mots volontairement peu nombreux : un mot qui pourrait désigner deux catégories rend le
// favori ambigu, et un favori ambigu n'est pas classé.
const CATEGORIES: &[Category] = &[
    Category {
        name: "Jeux", icon: "🎮", color: "#c084fc",
        synonyms: &["Jeux", "Jeu", "Jeux vidéo", "Games", "Game", "Gaming"],
        domains: JEUX,
        host_words: &["game", "games", "gaming*", "jeux*", "esport*"],
        title_words: &["jeu", "jeux", "game", "games", "gaming", "gamer", "esport", "esports", "speedrun", "minecraft", "pokemon"],
    },
    Category {
        name: "Vidéos", icon: "🎬", color: "#f87171",
        synonyms: &["Vidéos", "Vidéo", "Streaming", "Films", "Séries", "Films et séries", "Cinéma", "TV"],
        domains: VIDEOS,
        host_words: &["video*", "streaming", "film*", "cinema*", "anime*"],
        title_words: &["video", "videos", "film", "films", "serie", "series", "streaming", "cinema", "anime", "episode", "trailer"],
    },
    Category {
        name: "Musique", icon: "🎵", color: "#f472b6",
        synonyms: &["Musique", "Music", "Audio", "Radio"],
        domains: MUSIQUE,
        host_words: &["music*", "musique*", "radio", "lyrics*", "paroles*"],
        title_words: &["musique", "music", "radio", "playlist", "album", "lyrics", "paroles", "podcast"],
    },
    Category {
        name: "Réseaux", icon: "💬", color: "#818cf8",
        synonyms: &["Réseaux", "Réseaux sociaux", "Social", "Social media", "Communauté", "Forums"],
        domains: RESEAUX,
        host_words: &["forum*", "social"],
        title_words: &["forum", "forums", "communaute", "community"],
    },
    Category {
        name: "Dev", icon: "💻", color: "#34d399",
        synonyms: &["Dev", "Développement", "Code", "Programmation", "Programming", "Développeur", "Developer", "Informatique"],
        domains: DEV,
        host_words: &["docs", "developer*", "developers", "dev", "api", "git"],
        title_words: &["api", "sdk", "documentation", "developer", "developers", "developpeur", "programming", "programmation", "javascript", "typescript", "github"],
    },
    Category {
        name: "Achats", icon: "🛒", color: "#fb923c",
        synonyms: &["Achats", "Shopping", "Courses", "Boutique", "Shop"],
        domains: ACHATS,
        host_words: &["shop", "boutique*", "store"],
        title_words: &["boutique", "shop", "shopping", "achat", "achats", "promo", "promos", "soldes", "panier"],
    },
    Category {
        name: "Actualités", icon: "📰", color: "#cbd5e1",
        synonyms: &["Actualités", "Actualité", "Actus", "Actu", "News", "Infos", "Presse", "Journaux"],
        domains: ACTUALITES,
        host_words: &["news*", "actu*", "journal*"],
        title_words: &["actu", "actus", "actualite", "actualites", "news", "journal", "infos", "presse"],
    },
    Category {
        name: "Outils", icon: "🧰", color: "#38bdf8",
        synonyms: &["Outils", "Outil", "Tools", "Utilitaires", "Productivité", "Bureautique", "IA", "AI"],
        domains: OUTILS,
        host_words: &["mail", "webmail"],
        title_words: &["webmail", "email", "messagerie", "convertisseur", "converter", "pdf", "traduction", "traducteur", "translate", "calculatrice", "calculator", "chatbot"],
    },
    Category {
        name: "Banque et admin", icon: "🏦", color: "#d6a77a",
        synonyms: &["Banque et admin", "Banque", "Admin", "Administratif", "Administration", "Finances", "Finance", "Argent", "Papiers"],
        domains: BANQUE_ADMIN,
        host_words: &["bank*", "banque*", "credit*", "assurance*", "mutuelle*", "impots"],
        title_words: &["banque", "bank", "banking", "credit", "assurance", "mutuelle", "impot", "impots", "facture", "factures", "prefecture", "mairie", "administration"],
    },
    Category {
        name: "Savoir", icon: "📚", color: "#facc15",
        synonyms: &["Savoir", "Apprendre", "Éducation", "Cours", "École", "Études", "Culture", "Wiki"],
        domains: SAVOIR,
        host_words: &["wiki*", "learn*", "edu", "academy*"],
        title_words: &["wiki", "wikipedia", "cours", "tutoriel", "tutorial", "encyclopedie", "dictionnaire", "dictionary", "recette", "recettes", "recipe", "definition", "lecon"],
    },
    Category {
        name: "Voyages", icon: "✈️", color: "#2dd4bf",
        synonyms: &["Voyages", "Travel", "Vacances", "Transports"],
        domains: VOYAGES,
        host_words: &["travel*", "voyage*", "hotel*", "flight*", "airlines"],
        title_words: &["voyage", "voyages", "travel", "hotel", "hotels", "flight", "flights", "itineraire", "camping", "sejour", "vacances"],
    },
    Category {
        name: "Sport", icon: "⚽", color: "#a3e635",
        synonyms: &["Sport", "Foot", "Football"],
        domains: SPORT,
        host_words: &["sport*", "foot", "football", "rugby*", "tennis*", "basket*", "fitness"],
        title_words: &["sport", "sports", "football", "foot", "rugby", "tennis", "basket", "basketball", "nba", "fitness", "musculation", "running", "cyclisme", "olympique", "olympiques"],
    },
];

/// `loose` : (url, titre) des favoris sans dossier. `folders` : noms des dossiers existants.
pub fn suggest(loose: &[(String, String)], folders: &[String]) -> Vec<Proposal> {
    let mut buckets = vec![Vec::new(); CATEGORIES.len()];
    let mut seen = HashSet::new();
    for (url, title) in loose {
        let Some(cat) = host_of(url).and_then(|h| classify(&h, title)) else { continue };
        if seen.insert(url.as_str()) {
            buckets[cat].push(url.clone());
        }
    }
    let mut out: Vec<Proposal> = buckets
        .into_iter()
        .zip(CATEGORIES)
        .filter_map(|(urls, c)| {
            let existing = folders.iter().find(|f| equivalent(f, c));
            // Un dossier à créer pour un seul favori n'aide pas à ranger.
            if urls.len() < if existing.is_some() { 1 } else { 2 } {
                return None;
            }
            Some(Proposal {
                folder: existing.cloned().unwrap_or_else(|| c.name.to_string()),
                new: existing.is_none(),
                icon: c.icon,
                color: c.color,
                urls,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.urls.len().cmp(&a.urls.len())
            .then(a.new.cmp(&b.new))
            .then_with(|| fold_name(&a.folder).cmp(&fold_name(&b.folder)))
            .then_with(|| a.folder.cmp(&b.folder))
    });
    out
}

/// Hôte en minuscules d'une URL http(s), sans identifiants ni port ; `None` sinon.
fn host_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let host = match hostport.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?, // IPv6 : le « : » fait partie de l'adresse
        None => hostport.split(':').next()?,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn classify(host: &str, title: &str) -> Option<usize> {
    by_domain(host).or_else(|| by_words(host, title))
}

/// Catégorie du domaine connu le plus long qui couvre l'hôte.
fn by_domain(host: &str) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (i, c) in CATEGORIES.iter().enumerate() {
        for d in c.domains {
            let covers = host == *d || host.strip_suffix(d).is_some_and(|p| p.ends_with('.'));
            if covers && best.is_none_or(|(_, len)| d.len() > len) {
                best = Some((i, d.len()));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Repli par mots : l'hôte d'abord, puis le titre ; un seul candidat ou rien.
fn by_words(host: &str, title: &str) -> Option<usize> {
    let frags: Vec<&str> = host.split(['.', '-']).collect();
    let in_host = |w: &str| match w.strip_suffix('*') {
        Some(p) => frags.iter().any(|f| f.starts_with(p)),
        None => frags.contains(&w),
    };
    let folded = fold(title);
    let words: Vec<&str> = folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let in_title = |w: &str| words.contains(&w);
    for hits in [
        hits_of(|c| c.host_words.iter().any(|w| in_host(w))),
        hits_of(|c| c.title_words.iter().any(|w| in_title(w))),
    ] {
        match hits.as_slice() {
            [] => continue,
            [one] => return Some(*one),
            _ => return None, // plusieurs catégories possibles : on s'abstient
        }
    }
    None
}

fn hits_of(pred: impl Fn(&Category) -> bool) -> Vec<usize> {
    CATEGORIES.iter().enumerate().filter(|(_, c)| pred(c)).map(|(i, _)| i).collect()
}

fn equivalent(folder: &str, c: &Category) -> bool {
    let f = fold_name(folder);
    !f.is_empty() && (fold_name(c.name) == f || c.synonyms.iter().any(|s| fold_name(s) == f))
}

/// Minuscules sans accents.
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c => c,
        })
        .collect()
}

/// Forme canonique d'un nom de dossier : sans accents, casse, emoji ni pluriel final (« Jeux » = « jeu »).
fn fold_name(s: &str) -> String {
    let folded = fold(s);
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| if w.len() > 3 { w.strip_suffix(['s', 'x']).unwrap_or(w) } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(url: &str, title: &str) -> Option<&'static str> {
        host_of(url).and_then(|h| classify(&h, title)).map(|i| CATEGORIES[i].name)
    }

    fn loose(items: &[&str]) -> Vec<(String, String)> {
        items.iter().map(|u| (u.to_string(), String::new())).collect()
    }

    #[test]
    fn domaine_exact_et_sous_domaine() {
        assert_eq!(cat("https://lemonde.fr/", ""), Some("Actualités"));
        assert_eq!(cat("https://www.jeuxvideo.com/forums", ""), Some("Jeux"));
        assert_eq!(cat("http://fr.wikipedia.org/wiki/Rust", ""), Some("Savoir"));
        assert_eq!(cat("https://WWW.LeBonCoin.fr:443/annonces", ""), Some("Achats"));
        assert_eq!(cat("https://mabanque.bnpparibas/", ""), Some("Banque et admin"));
        assert_eq!(cat("https://pasdutoutlemonde.fr/", ""), None); // frontière de label respectée
    }

    #[test]
    fn entree_la_plus_precise_l_emporte() {
        assert_eq!(cat("https://music.youtube.com/watch", ""), Some("Musique"));
        assert_eq!(cat("https://www.youtube.com/watch", ""), Some("Vidéos"));
        assert_eq!(cat("https://gaming.amazon.com/", ""), Some("Jeux"));
        assert_eq!(cat("https://www.amazon.com/", ""), Some("Achats"));
        assert_eq!(cat("https://store.steampowered.com/app/1", ""), Some("Jeux"));
        assert_eq!(cat("https://mail.google.com/", ""), Some("Outils"));
        assert_eq!(cat("https://drive.google.com/", ""), Some("Outils"));
        assert_eq!(cat("https://news.google.com/", ""), Some("Actualités"));
        assert_eq!(cat("https://www.google.com/", ""), None);
        assert_eq!(cat("https://www.impots.gouv.fr/", ""), Some("Banque et admin"));
        assert_eq!(cat("https://www.legifrance.gouv.fr/", ""), Some("Banque et admin"));
        assert_eq!(cat("https://rmcsport.bfmtv.com/", ""), Some("Sport"));
        assert_eq!(cat("https://www.bfmtv.com/", ""), Some("Actualités"));
        assert_eq!(cat("https://aws.amazon.com/", ""), Some("Dev"));
    }

    #[test]
    fn repli_par_mots() {
        assert_eq!(cat("https://banque-exemple.fr/", ""), Some("Banque et admin"));
        assert_eq!(cat("https://shop.exemple.com/", ""), Some("Achats"));
        assert_eq!(cat("https://docs.exemple.io/", "Accueil"), Some("Dev"));
        assert_eq!(cat("https://exemple.org/", "Recette du gâteau au chocolat"), Some("Savoir"));
        assert_eq!(cat("https://exemple.org/", "Le JOURNAL du soir"), Some("Actualités"));
        // Le domaine connu passe avant les mots.
        assert_eq!(cat("https://www.marmiton.org/", "Actu cuisine"), Some("Savoir"));
    }

    #[test]
    fn dans_le_doute_on_ne_classe_pas() {
        assert_eq!(cat("https://exemple.org/", "Accueil"), None);
        assert_eq!(cat("https://photoshop-astuces.fr/", ""), None); // « shop » au milieu d'un mot
        assert_eq!(cat("https://exemple.org/", "Jeux olympiques 2028"), None); // Jeux et Sport
        assert_eq!(cat("https://game-shop.exemple/", ""), None); // hôte ambigu
    }

    #[test]
    fn urls_invalides_ignorees() {
        for u in ["javascript:alert(1)", "ftp://youtube.com/", "https://", "file:///C:/x.html", "chrome://settings", "youtube.com"] {
            assert_eq!(host_of(u).and_then(|h| classify(&h, "")), None, "{u}");
        }
        let p = suggest(&loose(&["ftp://youtube.com/a", "https://youtube.com/b", "https://youtu.be/c", "about:blank"]), &[]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].urls, ["https://youtube.com/b", "https://youtu.be/c"]);
    }

    #[test]
    fn dossier_existant_reutilise_sous_son_nom() {
        let folders = vec!["Perso".to_string(), "🎬 VIDEOS".to_string(), "jéux".to_string()];
        let p = suggest(&loose(&["https://www.youtube.com/a", "https://www.twitch.tv/b", "https://www.ign.com/"]), &folders);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].folder.as_str(), p[0].new, p[0].urls.len()), ("🎬 VIDEOS", false, 2));
        assert_eq!((p[1].folder.as_str(), p[1].new, p[1].urls.len()), ("jéux", false, 1)); // dès 1 favori
        assert!(equivalent("Développement", &CATEGORIES[4]));
        assert!(equivalent("PROGRAMMATION", &CATEGORIES[4]));
        assert!(!equivalent("Perso", &CATEGORIES[4]));
    }

    #[test]
    fn seuil_de_deux_pour_un_dossier_a_creer() {
        assert!(suggest(&loose(&["https://www.ign.com/"]), &[]).is_empty());
        let p = suggest(&loose(&["https://www.ign.com/", "https://www.roblox.com/"]), &[]);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].folder.as_str(), p[0].new, p[0].icon), ("Jeux", true, "🎮"));
    }

    #[test]
    fn ordre_des_propositions_et_unicite() {
        let urls = loose(&[
            "https://www.ign.com/", "https://youtube.com/1", "https://deezer.com/", "https://amazon.fr/",
            "https://youtube.com/2", "https://roblox.com/", "https://spotify.com/", "https://youtube.com/3",
            "https://ebay.fr/", "https://youtube.com/1", // doublon : une seule fois
        ]);
        let p = suggest(&urls, &["musique".to_string()]);
        let names: Vec<&str> = p.iter().map(|x| x.folder.as_str()).collect();
        assert_eq!(names, ["Vidéos", "musique", "Achats", "Jeux"]);
        assert_eq!(p[0].urls, ["https://youtube.com/1", "https://youtube.com/2", "https://youtube.com/3"]);
        assert_eq!(suggest(&urls, &["musique".to_string()]), p); // stable
    }

    #[test]
    fn tables_propres() {
        let mut all = HashSet::new();
        let mut syns = HashSet::new();
        let mut colors = HashSet::new();
        for c in CATEGORIES {
            assert!(c.domains.windows(2).all(|w| w[0] < w[1]), "{} : table non triée ou doublon", c.name);
            for d in c.domains {
                assert!(all.insert(*d), "{d} présent dans deux catégories");
                assert_eq!(*d, d.to_ascii_lowercase());
                assert!(!d.starts_with("www."), "{d}");
            }
            let mine: HashSet<String> = c.synonyms.iter().chain([&c.name]).map(|s| fold_name(s)).collect();
            for f in mine {
                assert!(syns.insert(f.clone()), "synonyme partagé entre catégories : {f}");
            }
            assert!(colors.insert(c.color) && c.color.len() == 7 && c.color.starts_with('#'));
        }
        assert!(CATEGORIES.len() <= 12);
    }
}
