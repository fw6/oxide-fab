/** The small vocabulary every panel draws with, sized for a dense dashboard.
 *
 * Values that come from a signal are passed as accessors (`text={() => …}`):
 * a component runs once, so a plain string prop would freeze at creation. The
 * kit calls the accessor inside the JSX expression, which is what makes the
 * native view follow the signal.
 */
import { Text, View } from "nsh";
import type { JSX } from "nsh";
import { DARK, LIGHT, type Palette } from "../palette";
import { dark, dense } from "../store";

export const TINY = 10.5;
export const SMALL = 11.5;
export const BODY = 12.5;
export const TITLE = 13.5;

export const FONT =
  "WenQuanYi Micro Hei, Noto Sans CJK SC, Source Han Sans SC, PingFang SC, Microsoft YaHei, sans-serif";

/** A value, or an accessor that reads the signals it depends on. */
export type Val<T> = T | (() => T);

function read<T>(value: Val<T>): T {
  return typeof value === "function" ? (value as () => T)() : value;
}

/** The palette now; reading it inside a style follows the theme toggle. */
export function p(): Palette {
  return dark() ? DARK : LIGHT;
}

/** A labelled panel; `source` is the subtitle the density toggle hides. */
export function Card(props: { title: string; source?: string; children?: JSX.Element }) {
  return (
    <View
      style={{
        width: "100%",
        padding: 12,
        gap: 8,
        borderRadius: 10,
        backgroundColor: p().surface,
        borderWidth: 1,
        borderColor: p().border,
      }}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: TITLE, fontWeight: "600", color: p().text }}>{props.title}</Text>
        <View style={{ flex: 1 }} />
        {dense() && (props.source ?? "") !== "" ? (
          <Text style={{ fontSize: TINY, color: p().muted, flexShrink: 1 }}>{props.source}</Text>
        ) : null}
      </View>
      {props.children}
    </View>
  );
}

/** A press target: two looks (secondary and accent), no hover or disabled states. */
export function Button(props: { label: Val<string>; accent?: boolean; onPress: () => void }) {
  return (
    <View
      onPress={props.onPress}
      style={{
        paddingHorizontal: 8,
        paddingVertical: 4,
        borderRadius: 6,
        borderWidth: 1,
        borderColor: p().border,
        backgroundColor: props.accent ? p().accent : p().surfaceAlt,
      }}
    >
      <Text style={{ fontSize: SMALL, color: props.accent ? p().accentText : p().text }}>{read(props.label)}</Text>
    </View>
  );
}

export function Small(props: { text: Val<string>; color?: Val<string> }) {
  return (
    <Text style={{ fontSize: SMALL, color: read(props.color ?? p().muted) }}>{read(props.text)}</Text>
  );
}

export function Strong(props: { text: Val<string>; color?: Val<string> }) {
  return (
    <Text style={{ fontSize: BODY, fontWeight: "600", color: read(props.color ?? p().text) }}>{read(props.text)}</Text>
  );
}

export function Dot(props: { color: Val<string> }) {
  return <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: read(props.color) }} />;
}

export function Badge(props: { label: Val<string>; fg: Val<string>; bg?: Val<string> }) {
  return (
    <View
      style={{
        paddingHorizontal: 6,
        paddingVertical: 2,
        borderRadius: 5,
        backgroundColor: read(props.bg ?? p().surfaceAlt),
      }}
    >
      <Text style={{ fontSize: TINY, color: read(props.fg) }}>{read(props.label)}</Text>
    </View>
  );
}

export function Kv(props: { key: Val<string>; value: Val<string> }) {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      <Text style={{ fontSize: SMALL, color: p().muted }}>{read(props.key)}</Text>
      <View style={{ flex: 1 }} />
      <Text style={{ fontSize: SMALL, color: p().text }}>{read(props.value)}</Text>
    </View>
  );
}

export function Bar(props: { ratio: Val<number>; color: Val<string>; width: number }) {
  return (
    <View style={{ height: 6, width: props.width, borderRadius: 3, backgroundColor: p().surfaceAlt }}>
      <View
        style={{
          height: 6,
          width: props.width * Math.min(1, Math.max(0, read(props.ratio))),
          borderRadius: 3,
          backgroundColor: read(props.color),
        }}
      />
    </View>
  );
}

export function Divider() {
  return <View style={{ width: "100%", height: 1, backgroundColor: p().border }} />;
}

export function SectionLabel(props: { label: Val<string> }) {
  return (
    <Text style={{ fontSize: TINY, fontWeight: "600", color: p().muted }}>{read(props.label)}</Text>
  );
}
