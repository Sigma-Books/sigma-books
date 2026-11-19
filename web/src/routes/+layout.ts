// Application compilée en fichiers statiques (adapter-static), servis par le backend Rust.
// - `prerender` : chaque page est une coquille HTML prérendue, qui porte la CSP en <meta>.
// - `ssr = false` : rien n'est rendu à la compilation ; les données viennent de l'API, dans le
//   navigateur, avec le cookie de session. Le cabinet et le dossier sont lus dans l'URL.
// - `trailingSlash = 'always'` : chaque page est un `index.html` de répertoire, que le backend
//   sert sans redirection (`crates/api/src/serveur/statique.rs`).
export const prerender = true;
export const ssr = false;
export const trailingSlash = 'always';
