// Sample data for the design mockups, using the real cached artwork.
// `design/art` is a junction to %LOCALAPPDATA%\gdash\data\art (serve this folder over http).
const ART = 'art/';
const art = (id, kind) => `${ART}${id}/${kind}.${kind === 'logo' || kind === 'icon' ? 'png' : 'jpg'}`;

const G = {
  bg3: { id: 'steam-1086940', title: "Baldur's Gate 3", system: 'Steam', played: 'Played 3 months ago', logo: true },
  hd2: { id: 'steam-553850', title: 'HELLDIVERS 2', system: 'Steam', played: 'Played 7 days ago', logo: true },
  nms: { id: 'steam-275850', title: "No Man's Sky", system: 'Steam', played: 'Played last month', logo: true },
  ck3: { id: 'steam-1158310', title: 'Crusader Kings III', system: 'Steam', played: 'Played last month', logo: true },
  abf: { id: 'steam-427410', title: 'Abiotic Factor', system: 'Steam', played: 'Played 10 months ago', logo: true },
  obl: { id: 'pc-the-elder-scrolls-iv-oblivion-remastered', title: 'The Elder Scrolls IV: Oblivion Remastered', system: 'PC', played: '', logo: false },
  ngd: { id: 'pc-northgard', title: 'Northgard', system: 'PC', played: '', logo: true },
  me1: { id: 'pc-mass-effect', title: 'Mass Effect', system: 'PC', played: '', logo: true },
  aom: { id: 'pc-age-of-mythology-retold', title: 'Age of Mythology: Retold', system: 'PC', played: '', logo: true },
  kot: { id: 'xbox-star-wars-knights-of-the-old-republic', title: 'Star Wars: Knights of the Old Republic', system: 'Xbox', played: '', logo: true },
  mor: { id: 'xbox-the-elder-scrolls-iii-morrowind-game-of-the-year-edition', title: 'The Elder Scrolls III: Morrowind', system: 'Xbox', played: '', logo: true },
  cc: { id: 'x360-castle-crashers', title: 'Castle Crashers', system: 'Xbox 360', played: 'Played 4 months ago', logo: true },
};
for (const g of Object.values(G)) g.sub = [g.system, g.played].filter(Boolean).join('  ·  ');

const BLADES = [
  { name: 'Recent', color: '#2fd0c0', games: [G.hd2, G.ck3, G.nms, G.bg3, G.cc, G.abf] },
  { name: 'Steam', color: '#4d9bff', games: [G.abf, G.bg3, G.ck3, G.hd2, G.nms] },
  { name: 'PC', color: '#ff9a3c', games: [G.aom, G.me1, G.ngd, G.obl] },
  { name: 'Xbox', color: '#6fdc3a', games: [G.kot, G.mor] },
  { name: 'Xbox 360', color: '#d65bd0', games: [G.cc] },
  { name: 'System', color: '#a7b4c4', games: [] },
];
BLADES[5].games = [];
BLADES.splice(5, 1); // System blade omitted from the art mockups
