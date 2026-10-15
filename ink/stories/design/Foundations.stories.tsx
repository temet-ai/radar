import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Board, card, intro, section, sectionsBut } from "./Board";
import { Shipped, Target, TargetLabel } from "./parts";

/**
 * Design foundations from the canvas: colour (board 01), cells and type (board 02) and icons (board 03).
 *
 * Each story is the canvas's own drawing. Parts the canvas marks **shipped** are today's look (the Nodes, Text and
 * Chrome stories show the real components; the toolbar's Tier switch draws any of them at 256, 16 or mono). Parts
 * marked **new in ink** are targets: "Target · not built yet" with the issue that builds them.
 */
const meta = {
  title: "Design/Foundations",
  component: Board,
  parameters: { layout: "fullscreen" },
  args: { name: "Colour" },
} satisfies Meta<typeof Board>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Colour slots with swatches, their 256 and 16 stand-ins and contrast on a panel (board 01). The slots are shipped;
 * the host sends them as the Look (see the Rules page for every slot at every tier as the host resolves it). */
export const ColourSlots: Story = {
  render: () => (
    <Shipped
      board="Colour"
      what="the theme's slots (SilkCircuit Neon)"
      picks={[intro, sectionsBut(["Ramps & tints", "One row, four tiers"])]}
    />
  ),
};

/** The glow ramp (one step per cell, for the focused border and the wordmark) and tone tints (a chip's ground in its
 * tone). */
export const GlowRampAndTints: Story = {
  render: () => <Target board="Colour" picks={[section("Ramps & tints")]} issues={[31, 30]} note="#31 the ramp, #30 the tints" />,
};

/** One row at the four colour tiers: the meaning survives every step down; at none, weight and underline carry it. */
export const FourTiers: Story = {
  render: () => <Shipped board="Colour" what="every slot has a 256 and a 16 stand-in" picks={[section("One row, four tiers")]} />,
};

/** The cell, and the hierarchy as drawn on an item page: every voice radar has, on one screen (board 02). */
export const CellAndHierarchy: Story = {
  render: () => <Shipped board="Type" what="the cell grid and the type hierarchy" picks={[intro, section("The cell"), section("The hierarchy, as drawn")]} />,
};

/** The attributes (bold, underline, italic, reverse shipped; undercurl and strike proposed) and punctuation. */
export const AttributesAndPunctuation: Story = {
  render: () => (
    <div>
      <TargetLabel issues={[47]} note="undercurl and strike (the rows marked new in ink below) have no issue of their own yet; the style guide, #47, orders them" />
      <Shipped board="Type" what="bold, underline, italic, reverse; punctuation" picks={[section("Attributes"), section("Punctuation")]} />
    </div>
  ),
};

/** Display type: big digits and a display face for the numbers that are the point of a screen. */
export const DisplayType: Story = {
  render: () => <Target board="Type" picks={[section("Display type")]} issues={[39]} note="big digits come with the charts" />,
};

/** Other faces: three ways to bring in another face, each for one place, each falling back to the terminal face. */
export const OtherFaces: Story = {
  render: () => <Target board="Type" picks={[section("Other faces")]} issues={[38]} note="a face drawn as a picture rides on the pictures work" />,
};

/** Every icon with its fallback, by group, and the packs (board 03). */
export const Icons: Story = {
  render: () => <Shipped board="Icons" what="the icon pack and its fallbacks" picks={[intro, sectionsBut(["Part marks"])]} />,
};

/** The fixed two-cell icon slot: changing pack never moves a column. */
export const FixedIconSlot: Story = {
  render: () => <Target board="Icons" picks={[card("in ink · a fixed two-cell icon slot")]} issues={[43]} />,
};

/** Part marks: each part's own mark in the top bar and on its rows. */
export const PartMarks: Story = {
  render: () => <Target board="Icons" picks={[section("Part marks")]} issues={[43]} note="drawn in the two-cell slot" />,
};
