# FalloutAVBrowser

Lecteur audio / vidéo pour les formats de Fallout 1/2 et d'Arcanum, en Rust. Même interface que FalloutImageBrowser :
arborescence à gauche (dossiers et archives `.DAT` parcourus comme des dossiers), média au centre, propriétés à droite.

## Formats

| Format | Contenu | Prise en charge |
|---|---|---|
| `.ACM` | audio Interplay (musiques, sons, voix) | lecture, export WAV |
| `.MVE` | vidéo Interplay MVE 8 bits (cinématiques Fallout) | lecture avec son (DPCM / PCM), export d'une image en PNG |
| `.BIK` | vidéo Bink 1 (Arcanum), révisions f à k | lecture avec son (RDFT / DCT), export d'une image en PNG |
| `.MP3` | audio (Arcanum, mods) | lecture, export WAV |
| `.WAV` | PCM 8/16/24/32 bits, flottant 32 bits | lecture, export |

Notes :

- Décodeur ACM : port de libacm (Marko Kreen, licence ISC).
- Décodeur Bink : port du décodeur de FFmpeg (LGPL 2.1 ou ultérieure) ; la révision `b` et Bink 2 ne sont pas gérés.
- Canaux ACM : les en-têtes de Fallout ne sont pas fiables. Mode « Automatique » : dossier `music` ou signal nettement
  stéréo → stéréo ; sinon mono. Réglable dans le menu Navigation.
- Archives : `.DAT` de Fallout 1 (DAT1) et 2 (DAT2), et archives Troika d'Arcanum et de Temple of Elemental Evil
  (`.dat`, `.patch0` à `.patch9`, signature `1TAD`). Les archives `.zip` et `.7z` se parcourent de la même façon
  (ZIP : deflate, deflate64, LZMA ; 7z : LZMA, LZMA2, deflate, BCJ ; pas de mot de passe ; une archive 7z « solide » se
  décompresse depuis le début de son bloc pour lire un fichier).
- Arborescence : barre de 8 accès rapides sous le filtre (Bureau, Documents, Téléchargements réservés + 5 favoris : clic
  droit sur un dossier → Favoris, clic droit sur un favori → retirer) ; saisie au clavier au-dessus de l'arborescence pour
  sauter à un nom ; clic droit → Actualiser (F5) ; SHA-256 dans les propriétés (copier, puis ouvrir un autre fichier : le
  panneau indique s'il est identique).
- Conversion (Fichier → Convertir le fichier…, Ctrl+E) : audio en WAV 8 / 16 bits, WAV flottant ou PCM brut (fréquence,
  canaux, normalisation, plage), image courante ou séquence PNG d'une vidéo, ou copie du fichier d'origine.
- Pas encore pris en charge : MVE 16 bits, sous-titres `.SVE`.

## Utilisation

```text
fallout_av_browser [dossier | fichier | archive.dat]
fallout_av_browser --info  <fichier>
fallout_av_browser --ls    <archive.dat> [filtre]
fallout_av_browser --convert <fichier> <wav16|wav8|f32|raw|frame|seq|orig> <dossier> [fréquence] [canaux] [normaliser]
fallout_av_browser --wav   <fichier> <sortie.wav>
fallout_av_browser --frame <fichier.mve|bik> <n> <sortie.png>
```

Raccourcis : Espace lecture / pause, ←/→ ±5 s, ↑/↓ fichier précédent / suivant, F11 plein écran, T / I arborescence et
propriétés. Glisser-déposer d'un fichier ou dossier. Voir *Aide → À propos*.

## Compilation

```text
cargo build --release
```
