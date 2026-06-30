// Camelot key to hue mapping for color-coded key display.

const KEY_HUES = {
  // Camelot notation
  "1A":  0,    "1B":  0,
  "2A":  30,   "2B":  30,
  "3A":  55,   "3B":  55,
  "4A":  85,   "4B":  85,
  "5A":  120,  "5B":  120,
  "6A":  155,  "6B":  155,
  "7A":  180,  "7B":  180,
  "8A":  200,  "8B":  200,
  "9A":  230,  "9B":  230,
  "10A": 260,  "10B": 260,
  "11A": 290,  "11B": 290,
  "12A": 325,  "12B": 325,
  // Standard notation (minor = A, major = B)
  "ABM": 0,    "B":   0,
  "EBM": 30,   "F#":  30,  "GB":  30,
  "BBM": 55,   "DB":  55,  "C#":  55,
  "FM":  85,   "AB":  85,  "G#":  85,
  "CM":  120,  "EB":  120, "D#":  120,
  "GM":  155,  "BB":  155, "A#":  155,
  "DM":  180,  "F":   180,
  "AM":  200,  "C":   200,
  "EM":  230,  "G":   230,
  "BM":  260,  "D":   260,
  "F#M": 290,  "GBM": 290, "A":  290,
  "C#M": 325,  "DBM": 325, "E":  325,
};

export function getKeyHue(key) {
  if (!key) return 270;
  const normalized = String(key).toUpperCase().replace(/\s+/g, "");
  return KEY_HUES[normalized] ?? 270;
}
