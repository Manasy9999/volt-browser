// Lecture des données d'un autre navigateur Chromium (Chrome, Edge, Brave, Vivaldi, Opera, Opera GX) pour les reprendre
// dans Volt : cookies, historique, mots de passe. Tout est sur la machine de l'utilisateur, pour ses propres profils.
//
// Les fichiers (Cookies, History, Login Data) sont des bases SQLite. Plutôt qu'une grosse dépendance C, un petit lecteur
// SQLite en lecture seule suffit : il parcourt l'arbre d'une table et rend ses lignes. Les valeurs sensibles (cookies,
// mots de passe) sont chiffrées ; elles ne sont déchiffrées qu'au moment de l'import et ne passent jamais à l'interface.
//
// shortcut: lecteur SQLite réduit aux tables simples de Chromium (pas d'index, pas de WITHOUT ROWID) ; à compléter si un
// navigateur change de schéma. Les bases doivent être copiées avant lecture (le navigateur source verrouille l'original).

use windows::Win32::Security::Cryptography::{
    BCRYPT_AES_ALGORITHM, BCRYPT_ALG_HANDLE, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO, BCRYPT_CHAIN_MODE_GCM, BCRYPT_CHAINING_MODE, BCRYPT_FLAGS,
    BCRYPT_KEY_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptGenerateSymmetricKey,
    BCryptOpenAlgorithmProvider, BCryptSetProperty,
};

/// Valeur d'une colonne SQLite.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Val {
    pub fn int(&self) -> i64 {
        match self {
            Val::Int(n) => *n,
            _ => 0,
        }
    }
    pub fn text(&self) -> &str {
        match self {
            Val::Text(s) => s,
            _ => "",
        }
    }
    pub fn blob(&self) -> &[u8] {
        match self {
            Val::Blob(b) => b,
            Val::Text(s) => s.as_bytes(),
            _ => &[],
        }
    }
}

/// Entier de longueur variable SQLite (gros-boutiste, 7 bits par octet, 9e octet complet). Rend (valeur, octets lus).
fn varint(b: &[u8]) -> (i64, usize) {
    let mut v: u64 = 0;
    for i in 0..8 {
        let Some(&x) = b.get(i) else { return (v as i64, i) };
        v = (v << 7) | (x & 0x7f) as u64;
        if x & 0x80 == 0 {
            return (v as i64, i + 1);
        }
    }
    // 9e octet : ses 8 bits sont gardés entiers
    let last = b.get(8).copied().unwrap_or(0);
    (((v << 8) | last as u64) as i64, 9)
}

struct Db<'a> {
    data: &'a [u8],
    page: usize,  // taille d'une page
    usable: usize, // page moins l'éventuelle réserve de fin
}

impl<'a> Db<'a> {
    fn open(data: &'a [u8]) -> Option<Db<'a>> {
        if !data.starts_with(b"SQLite format 3\0") {
            return None;
        }
        let page = match u16::from_be_bytes([data[16], data[17]]) {
            1 => 65536,
            n => n as usize,
        };
        if page < 512 || !page.is_power_of_two() || data.len() < page {
            return None;
        }
        let reserve = data[20] as usize;
        Some(Db { data, page, usable: page - reserve })
    }

    fn page(&self, n: usize) -> Option<&'a [u8]> {
        // les pages sont numérotées à partir de 1
        let start = (n.checked_sub(1)?).checked_mul(self.page)?;
        self.data.get(start..start + self.page)
    }

    /// Octets d'une charge utile, avec le suivi des pages de débordement si elle dépasse la cellule.
    fn payload(&self, total: usize, local: &[u8], first_overflow: Option<usize>) -> Vec<u8> {
        let mut out = local.to_vec();
        let mut next = first_overflow;
        // chaque page de débordement : 4 octets = page suivante (0 = fin), puis des données
        while out.len() < total {
            let Some(p) = next.filter(|&p| p != 0).and_then(|p| self.page(p)) else { break };
            next = Some(u32::from_be_bytes([p[0], p[1], p[2], p[3]]) as usize);
            let take = (total - out.len()).min(self.usable - 4);
            match p.get(4..4 + take) {
                Some(s) => out.extend_from_slice(s),
                None => break,
            }
        }
        out.truncate(total);
        out
    }

    /// Combien de la charge utile tient dans la cellule (le reste va en débordement). Formule du format SQLite.
    fn local_len(&self, total: usize) -> usize {
        let max = self.usable - 35;
        if total <= max {
            return total;
        }
        let min = (self.usable - 12) * 32 / 255 - 23;
        let k = min + (total - min) % (self.usable - 4);
        if k <= max { k } else { min }
    }

    /// Décode un enregistrement (en-tête de types + valeurs) en colonnes.
    fn record(&self, payload: &[u8], rowid: i64) -> Vec<Val> {
        let (hlen, mut p) = varint(payload);
        let hend = hlen as usize;
        let mut types = vec![];
        while p < hend {
            let (t, n) = varint(&payload[p..]);
            types.push(t);
            p += n;
        }
        let mut out = vec![];
        for t in types {
            let (val, len): (Val, usize) = match t {
                0 => (Val::Null, 0), // sera remplacé par le rowid si c'est la clé
                1 => (Val::Int(i8::from_be_bytes([g(payload, p)]) as i64), 1),
                2 => (Val::Int(i16::from_be_bytes([g(payload, p), g(payload, p + 1)]) as i64), 2),
                3 => (Val::Int(i24(&payload[p..])), 3),
                4 => (Val::Int(i32::from_be_bytes(a4(payload, p)) as i64), 4),
                5 => (Val::Int(i48(&payload[p..])), 6),
                6 => (Val::Int(i64::from_be_bytes(a8(payload, p))), 8),
                7 => (Val::Real(f64::from_be_bytes(a8(payload, p))), 8),
                8 => (Val::Int(0), 0),
                9 => (Val::Int(1), 0),
                n if n >= 12 && n % 2 == 0 => {
                    let l = (n as usize - 12) / 2;
                    (Val::Blob(payload.get(p..p + l).unwrap_or_default().to_vec()), l)
                }
                n if n >= 13 => {
                    let l = (n as usize - 13) / 2;
                    (Val::Text(String::from_utf8_lossy(payload.get(p..p + l).unwrap_or_default()).into_owned()), l)
                }
                _ => (Val::Null, 0),
            };
            out.push(val);
            p += len;
        }
        // une colonne INTEGER PRIMARY KEY est stockée NULL dans l'enregistrement : sa valeur est le rowid
        if let Some(first) = out.first_mut()
            && *first == Val::Null
        {
            *first = Val::Int(rowid);
        }
        out
    }

    /// Toutes les lignes d'une table à partir de sa page racine (parcours de l'arbre, feuilles 0x0d, nœuds 0x05).
    fn rows(&self, root: usize) -> Vec<Vec<Val>> {
        let mut out = vec![];
        self.walk(root, &mut out, 0);
        out
    }

    fn walk(&self, n: usize, out: &mut Vec<Vec<Val>>, depth: u32) {
        if depth > 64 {
            return; // garde-fou contre un fichier malformé qui bouclerait
        }
        let Some(page) = self.page(n) else { return };
        // la toute première page commence après l'en-tête de 100 octets du fichier
        let base = if n == 1 { 100 } else { 0 };
        let kind = page[base];
        let ncell = u16::from_be_bytes([page[base + 3], page[base + 4]]) as usize;
        let header = if kind == 0x05 { 12 } else { 8 };
        let ptrs = base + header;
        for i in 0..ncell {
            let Some(off) = page.get(ptrs + 2 * i..ptrs + 2 * i + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize) else { continue };
            match kind {
                0x0d => {
                    // feuille de table : charge utile (taille, rowid, données), débordement possible
                    let (total, l1) = varint(&page[off..]);
                    let (rowid, l2) = varint(&page[off + l1..]);
                    let start = off + l1 + l2;
                    let total = total as usize;
                    let local = self.local_len(total);
                    let overflow = if local < total {
                        page.get(start + local..start + local + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
                    } else {
                        None
                    };
                    let bytes = self.payload(total, page.get(start..start + local).unwrap_or_default(), overflow);
                    out.push(self.record(&bytes, rowid));
                }
                0x05 => {
                    // nœud interne : page enfant à gauche de la clé
                    if let Some(s) = page.get(off..off + 4) {
                        self.walk(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize, out, depth + 1);
                    }
                }
                _ => {}
            }
        }
        // nœud interne : enfant le plus à droite
        if kind == 0x05
            && let Some(s) = page.get(base + 8..base + 12)
        {
            self.walk(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize, out, depth + 1);
        }
    }
}

fn g(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}
fn a4(b: &[u8], p: usize) -> [u8; 4] {
    [g(b, p), g(b, p + 1), g(b, p + 2), g(b, p + 3)]
}
fn a8(b: &[u8], p: usize) -> [u8; 8] {
    std::array::from_fn(|i| g(b, p + i))
}
fn i24(b: &[u8]) -> i64 {
    let u = (g(b, 0) as i64) << 16 | (g(b, 1) as i64) << 8 | g(b, 2) as i64;
    (u ^ 0x80_0000).wrapping_sub(0x80_0000) // signé sur 24 bits
}
fn i48(b: &[u8]) -> i64 {
    let mut u = 0i64;
    for i in 0..6 {
        u = (u << 8) | g(b, i) as i64;
    }
    (u ^ 0x8000_0000_0000).wrapping_sub(0x8000_0000_0000)
}

/// Nom des colonnes d'une table, extrait du « CREATE TABLE … (col1 …, col2 …) » (pour retrouver une colonne par son
/// nom, l'ordre changeant d'une version de Chromium à l'autre).
fn columns(sql: &str) -> Vec<String> {
    let Some(open) = sql.find('(') else { return vec![] };
    let inner = &sql[open + 1..sql.rfind(')').unwrap_or(sql.len())];
    let mut cols = vec![];
    let mut depth = 0;
    for part in split_top(inner, &mut depth) {
        let p = part.trim();
        let first = p.split([' ', '\t', '\n', '(']).next().unwrap_or("");
        let name = first.trim_matches(['"', '`', '[', ']', '\'']);
        // sauter les contraintes de table (PRIMARY KEY(...), UNIQUE(...), etc.)
        if !name.is_empty() && !matches!(name.to_ascii_uppercase().as_str(), "PRIMARY" | "UNIQUE" | "CHECK" | "FOREIGN" | "CONSTRAINT" | "KEY") {
            cols.push(name.to_string());
        }
    }
    cols
}

/// Découpe sur les virgules de premier niveau (hors parenthèses).
fn split_top<'a>(s: &'a str, _d: &mut i32) -> Vec<&'a str> {
    let (mut out, mut depth, mut start) = (vec![], 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Une table d'un fichier SQLite Chromium : ses lignes, accessibles par nom de colonne.
pub struct Table {
    cols: Vec<String>,
    pub rows: Vec<Vec<Val>>,
}

impl Table {
    pub fn col(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.eq_ignore_ascii_case(name))
    }
    /// Valeur d'une ligne par nom de colonne (Null si absente).
    pub fn get<'r>(&self, row: &'r [Val], name: &str) -> &'r Val {
        self.col(name).and_then(|i| row.get(i)).unwrap_or(&Val::Null)
    }
}

/// Lit une table nommée d'un fichier SQLite déjà en mémoire. None si le fichier ou la table est illisible.
pub fn read_table(data: &[u8], table: &str) -> Option<Table> {
    let db = Db::open(data)?;
    // sqlite_master est la table de la page 1 : (type, name, tbl_name, rootpage, sql)
    for row in db.rows(1) {
        if row.first().map(Val::text) == Some("table") && row.get(1).map(Val::text) == Some(table) {
            let root = row.get(3)?.int() as usize;
            let cols = columns(row.get(4).map(Val::text).unwrap_or(""));
            if root == 0 {
                return None;
            }
            return Some(Table { cols, rows: db.rows(root) });
        }
    }
    None
}

// ===== déchiffrement des valeurs (cookies, mots de passe) =====
//
// Chromium chiffre avec une clé AES-256 posée dans « Local State » (champ os_crypt.encrypted_key), elle-même protégée
// par DPAPI (liée à la session Windows). Chaque valeur : préfixe « v10 »/« v11 » → AES-256-GCM (12 octets d'IV, puis le
// chiffré et 16 octets d'authentification) ; préfixe « v20 » → chiffrement « app-bound » (Chrome récent) qu'un autre
// programme ne peut pas défaire ; sinon, l'ancienne forme, directement DPAPI.

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = vec![];
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        acc = (acc << 6) | val(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Clé maître du profil Chromium (os_crypt.encrypted_key de « Local State », déprotégée par DPAPI).
pub fn master_key(local_state: &[u8]) -> Option<Vec<u8>> {
    let v: serde_json::Value = crate::parse_json(local_state)?;
    let b64 = v["os_crypt"]["encrypted_key"].as_str()?;
    let raw = b64_decode(b64)?;
    // les 5 premiers octets « DPAPI » marquent le schéma, le reste est la clé protégée
    let enc = raw.strip_prefix(b"DPAPI")?;
    crate::vault::unprotect(enc).filter(|k| k.len() == 32)
}

/// AES-256-GCM : rend le clair, ou None si l'authentification échoue (clé fausse, données altérées).
fn gcm_decrypt(key: &[u8], nonce: &[u8], ciphertext: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    let run = || -> windows::core::Result<Vec<u8>> {
        unsafe {
            let mut alg = BCRYPT_ALG_HANDLE::default();
            BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, None, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).ok()?;
            let inner = (|| {
                // mode GCM : la valeur est la chaîne large terminée par un zéro, passée en octets
                let mode: Vec<u16> = BCRYPT_CHAIN_MODE_GCM.as_wide().iter().copied().chain([0]).collect();
                BCryptSetProperty(alg.into(), BCRYPT_CHAINING_MODE, std::slice::from_raw_parts(mode.as_ptr() as *const u8, mode.len() * 2), 0).ok()?;
                let mut hkey = BCRYPT_KEY_HANDLE::default();
                BCryptGenerateSymmetricKey(alg, &mut hkey, None, key, 0).ok()?;
                let mut info = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
                    cbSize: size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
                    dwInfoVersion: 1,
                    pbNonce: nonce.as_ptr() as *mut u8,
                    cbNonce: nonce.len() as u32,
                    pbTag: tag.as_ptr() as *mut u8,
                    cbTag: tag.len() as u32,
                    ..Default::default()
                };
                let mut out = vec![0u8; ciphertext.len()];
                let mut n = 0u32;
                let d = BCryptDecrypt(hkey, Some(ciphertext), Some(&mut info as *mut _ as *const std::ffi::c_void), None, Some(&mut out), &mut n, BCRYPT_FLAGS(0)).ok();
                let _ = BCryptDestroyKey(hkey);
                d?;
                out.truncate(n as usize);
                Ok(out)
            })();
            let _ = BCryptCloseAlgorithmProvider(alg, 0);
            inner
        }
    };
    run().ok()
}

/// Déchiffre une valeur Chromium avec la clé maître. None si « app-bound » (v20) ou échec.
pub fn decrypt(value: &[u8], key: &[u8]) -> Option<Vec<u8>> {
    match value.get(..3) {
        Some(b"v10" | b"v11") if value.len() >= 3 + 12 + 16 => {
            let nonce = &value[3..15];
            let tag = &value[value.len() - 16..];
            gcm_decrypt(key, nonce, &value[15..value.len() - 16], tag)
        }
        Some(b"v20") => None, // app-bound (Chrome ≥ 127) : indéchiffrable hors du navigateur source
        _ if !value.is_empty() => crate::vault::unprotect(value), // ancienne forme : DPAPI direct
        _ => None,
    }
}

/// Valeur de cookie déchiffrée en texte. Les Chromium récents préfixent le clair de 32 octets (empreinte du domaine) :
/// retirés si le début n'est pas du texte et que la suite en est.
pub fn cookie_value(plain: Vec<u8>) -> String {
    let body = if plain.len() > 32 && std::str::from_utf8(&plain[..32]).is_err() && std::str::from_utf8(&plain[32..]).is_ok() {
        &plain[32..]
    } else {
        &plain[..]
    };
    String::from_utf8_lossy(body).into_owned()
}

// ===== extraction par type de donnée =====
// Chaque fonction reçoit le contenu du fichier SQLite déjà copié (le navigateur source verrouille l'original).

/// Un cookie prêt pour le gestionnaire de WebView2.
#[derive(Debug, Clone, PartialEq)]
pub struct Cookie {
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: i32, // 0 none, 1 lax, 2 strict (valeurs de WebView2) ; Chromium : -1/0/1/2
    pub expires: f64,   // secondes depuis 1970 ; 0 = cookie de session
}

/// Microsecondes depuis le 1er janvier 1601 (horodatage Chromium) -> secondes depuis 1970 (0 si session/invalide).
fn to_unix(utc: i64) -> f64 {
    if utc <= 0 {
        return 0.0;
    }
    (utc as f64 / 1_000_000.0) - 11_644_473_600.0
}

/// Cookies d'un fichier « Cookies » Chromium, déchiffrés avec la clé maître. Les valeurs vides ou indéchiffrables
/// (app-bound) sont ignorées.
pub fn parse_cookies(db: &[u8], key: &[u8]) -> Vec<Cookie> {
    let Some(t) = read_table(db, "cookies") else { return vec![] };
    let mut out = vec![];
    for r in &t.rows {
        let enc = t.get(r, "encrypted_value").blob();
        let plain = t.get(r, "value").text();
        let value = if !enc.is_empty() {
            match decrypt(enc, key) {
                Some(p) => cookie_value(p),
                None => continue, // app-bound ou échec : on saute ce cookie
            }
        } else {
            plain.to_string()
        };
        let host = t.get(r, "host_key").text();
        if host.is_empty() {
            continue;
        }
        // samesite Chromium -1 (non précisé) -> 0 (none) pour WebView2
        let ss = t.get(r, "samesite").int() as i32;
        out.push(Cookie {
            host: host.to_string(),
            name: t.get(r, "name").text().to_string(),
            value,
            path: {
                let p = t.get(r, "path").text();
                if p.is_empty() { "/".into() } else { p.to_string() }
            },
            secure: t.get(r, "is_secure").int() != 0,
            http_only: t.get(r, "is_httponly").int() != 0,
            same_site: ss.clamp(0, 2),
            expires: to_unix(t.get(r, "expires_utc").int()),
        });
    }
    out
}

/// Entrées d'historique (adresse, titre, nombre de visites, heure unix de dernière visite) d'un fichier « History ».
pub fn parse_history(db: &[u8]) -> Vec<(String, String, i64, f64)> {
    let Some(t) = read_table(db, "urls") else { return vec![] };
    t.rows
        .iter()
        .filter_map(|r| {
            let url = t.get(r, "url").text();
            url.starts_with("http").then(|| (url.to_string(), t.get(r, "title").text().to_string(), t.get(r, "visit_count").int(), to_unix(t.get(r, "last_visit_time").int())))
        })
        .collect()
}

/// Vrai si une valeur chiffrée de cette table/colonne est « app-bound » (préfixe v20, Chrome récent), donc ignorée.
pub fn app_bound_present(db: &[u8], table: &str, col: &str) -> bool {
    read_table(db, table).is_some_and(|t| t.rows.iter().any(|r| t.get(r, col).blob().starts_with(b"v20")))
}

/// Identifiants (site, utilisateur, mot de passe) d'un fichier « Login Data », déchiffrés. Les entrées sans mot de
/// passe lisible (app-bound) sont ignorées.
pub fn parse_logins(db: &[u8], key: &[u8]) -> Vec<(String, String, String)> {
    let Some(t) = read_table(db, "logins") else { return vec![] };
    let mut out = vec![];
    for r in &t.rows {
        let origin = t.get(r, "origin_url").text();
        let user = t.get(r, "username_value").text();
        let Some(pass) = decrypt(t.get(r, "password_value").blob(), key).filter(|p| !p.is_empty()) else { continue };
        if origin.starts_with("http") {
            out.push((origin.to_string(), user.to_string(), String::from_utf8_lossy(&pass).into_owned()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        assert_eq!(varint(&[0x00]), (0, 1));
        assert_eq!(varint(&[0x7f]), (127, 1));
        assert_eq!(varint(&[0x81, 0x00]), (128, 2));
        assert_eq!(varint(&[0x82, 0x2c]), (300, 2));
        // 9 octets : valeur maximale
        assert_eq!(varint(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]), (-1, 9));
    }

    // Lecture d'une vraie base SQLite au format Chromium, fabriquée par un script (VOLT_SQLITE_FIXTURE = son chemin).
    // Vérifie l'arbre, les types, les pages de débordement et l'accès par nom de colonne. Ignoré sans la variable.
    #[test]
    fn reads_chromium_db() {
        let Ok(path) = std::env::var("VOLT_SQLITE_FIXTURE") else { return };
        let data = std::fs::read(&path).unwrap();
        let t = read_table(&data, "cookies").expect("table cookies lue");
        assert!(t.col("host_key").is_some() && t.col("name").is_some() && t.col("encrypted_value").is_some());
        // 1000 lignes : l'arbre a plusieurs pages et au moins un nœud interne
        assert_eq!(t.rows.len(), 1000, "nombre de cookies");
        let find = |host: &str| t.rows.iter().find(|r| t.get(r, "host_key").text() == host).unwrap();
        let g = find(".google.com");
        assert_eq!(t.get(g, "name").text(), "SID");
        assert_eq!(t.get(g, "is_secure").int(), 1);
        // grosse valeur (débordement sur plusieurs pages) rendue intacte
        let big = find(".big.example");
        assert_eq!(t.get(big, "encrypted_value").blob(), vec![0xABu8; 9000]);
        // caractères accentués conservés (UTF-8)
        assert_eq!(t.get(find(".accent.fr"), "value").text(), "café-déjà-où");
        // entier négatif et grand entier
        assert_eq!(t.get(find(".neg.test"), "expires_utc").int(), -5);
        assert_eq!(t.get(find(".big.int"), "expires_utc").int(), 13_370_000_000_000_000);
    }

    // Pipeline complet sur un faux profil Chromium (VOLT_PROFILE_FIXTURE = son dossier) : clé maître DPAPI, cookies v10
    // (dont un à préfixe de 32 octets et un « app-bound » v20 ignoré, un ancien DPAPI), historique, mots de passe.
    // Le script `mkprofile.py` l'a fabriqué ; DPAPI étant lié à la session, le round-trip prouve le vrai déchiffrement.
    #[test]
    fn whole_profile() {
        let Ok(dir) = std::env::var("VOLT_PROFILE_FIXTURE").map(std::path::PathBuf::from) else { return };
        let exp: serde_json::Value = crate::parse_json(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();
        let key = master_key(&std::fs::read(dir.join("Local State")).unwrap()).expect("clé maître déprotégée");
        assert_eq!(hex(&key), exp["master_key_hex"].as_str().unwrap(), "clé maître");

        let cookies = parse_cookies(&std::fs::read(dir.join("Default/Network/Cookies")).unwrap(), &key);
        let find = |h: &str| cookies.iter().find(|c| c.host == h);
        // cookie v10 simple
        assert_eq!(find(".simple.com").unwrap().value, "jeton-simple-123");
        // cookie v10 avec préfixe de 32 octets (Chrome récent) : préfixe retiré
        assert_eq!(find(".prefixed.com").unwrap().value, "valeur-après-préfixe");
        // attributs transmis
        let s = find(".secure.com").unwrap();
        assert!(s.secure && s.http_only && (s.same_site == 1) && (s.expires > 1_700_000_000.0));
        // ancien cookie DPAPI direct
        assert_eq!(find(".legacy.com").unwrap().value, "ancienne-valeur");
        // cookie app-bound (v20) : ignoré, pas d'erreur
        assert!(find(".appbound.com").is_none(), "v20 ignoré");
        assert_eq!(cookies.len(), exp["cookie_count"].as_u64().unwrap() as usize);

        let hist = parse_history(&std::fs::read(dir.join("Default/History")).unwrap());
        assert!(hist.iter().any(|(u, t, n, at)| u == "https://exemple.fr/page" && t == "Une page" && *n == 7 && *at > 1_700_000_000.0));
        assert!(!hist.iter().any(|(u, ..)| u.starts_with("chrome://")), "les adresses internes sont écartées");

        let logins = parse_logins(&std::fs::read(dir.join("Default/Login Data")).unwrap(), &key);
        assert!(logins.contains(&("https://site.fr/login".into(), "jean".into(), "motdepasse!é".into())));
        assert!(!logins.iter().any(|(o, ..)| o.contains("appbound")), "mot de passe v20 ignoré");
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn column_names() {
        assert_eq!(columns("CREATE TABLE cookies (creation_utc INTEGER, host_key TEXT, name TEXT)"), ["creation_utc", "host_key", "name"]);
        let c = columns("CREATE TABLE \"logins\" (origin_url VARCHAR, username_value, password_value BLOB, UNIQUE(x,y), PRIMARY KEY (origin_url))");
        assert_eq!(c, ["origin_url", "username_value", "password_value"]);
    }
}
