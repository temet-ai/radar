// Emoji short names as `:name:` -> glyph, for the names chat and code hosts use most. An unknown name stays as written.
const EMOJI: Readonly<Record<string, string>> = {
  "+1": "👍", "-1": "👎", thumbsup: "👍", thumbsdown: "👎", heart: "❤️", tada: "🎉", rocket: "🚀", eyes: "👀",
  smile: "😄", grinning: "😀", joy: "😂", thinking_face: "🤔", pray: "🙏", clap: "👏", fire: "🔥", warning: "⚠️",
  white_check_mark: "✅", heavy_check_mark: "✔️", x: "❌", question: "❓", bug: "🐛", sparkles: "✨", memo: "📝",
  star: "⭐", zap: "⚡", wave: "👋", raised_hands: "🙌", point_up: "☝️", ok_hand: "👌", muscle: "💪", hourglass: "⏳",
  lock: "🔒", key: "🔑", bulb: "💡", construction: "🚧", package: "📦", link: "🔗", pushpin: "📌", boom: "💥",
};

/** `:tada:` -> 🎉; text with no short name, or an unknown one, comes back unchanged. */
export function withEmoji(text: string): string {
  return text.replace(/:([a-z0-9_+-]+):/g, (whole, name: string) => EMOJI[name] ?? whole);
}

export function emojiOf(name: string): string | undefined {
  return EMOJI[name.replace(/^:|:$/g, "")];
}
