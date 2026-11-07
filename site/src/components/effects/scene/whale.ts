// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import { BufferAttribute, BufferGeometry, Group, Mesh, SphereGeometry, Vector3, type Material } from "three";
import { BODY_LONG, MASCOT_R } from "./scene-config";

/**
 * WEB-5 O11: the toy whale's tail and flippers. The owner rejected O8's long
 * realistic body ("ini lebih kek dugong") and asked for a simple, chubby,
 * toy-like whale. The body itself is the mascot's one round shape (the
 * mark's dome on top, mascot.ts); this module adds what makes it a whale:
 * - a short tail stalk that leaves the back of the body and curls up to a
 *   small two-lobed fluke, which wags slowly;
 * - two small rounded flippers low on the sides, which paddle.
 * Everything is built once and animated by transforms only: no geometry is
 * rebuilt per frame. The scene draws these parts only in the picture below
 * the water, so above it the visitor still sees only the dome; by the time
 * the camera is under, the whale has dived with it (mascot.ts), so the tail
 * can curl up above its back.
 */

export type ToyParts = {
  group: Group;
  /** `swim` 0..1 is how much the whale is swimming (under water) rather than resting. */
  update: (t: number, swim: number, still: boolean) => void;
  dispose: () => void;
};

const TAU = Math.PI * 2;
/** One wag of the tail, in seconds: slow and happy. */
const WAG = 2.8;

/** The tail stalk's centre line: from inside the back of the body, down a little, then curling up. */
const TAIL = {
  root: new Vector3(0, -0.38, -1.15 * BODY_LONG),
  bend: new Vector3(0, -0.62, -2.15 * BODY_LONG),
  tip: new Vector3(0, 0.3, -2.3 * BODY_LONG),
};

function bezier(a: Vector3, b: Vector3, c: Vector3, t: number, out: Vector3): Vector3 {
  const u = 1 - t;
  return out
    .copy(a)
    .multiplyScalar(u * u)
    .addScaledVector(b, 2 * u * t)
    .addScaledVector(c, t * t);
}

/** A tapered tube along the tail's curve, with smooth normals, built once. */
function buildStalk(lowQuality: boolean): BufferGeometry {
  const rings = lowQuality ? 12 : 18;
  const sides = lowQuality ? 14 : 20;
  const pos: number[] = [];
  const nrm: number[] = [];
  const index: number[] = [];
  const c = new Vector3();
  const next = new Vector3();
  const T = new Vector3();
  const side = new Vector3();
  const vert = new Vector3();
  const n = new Vector3();
  const x = new Vector3(1, 0, 0);
  for (let i = 0; i < rings; i++) {
    const t = i / (rings - 1);
    bezier(TAIL.root, TAIL.bend, TAIL.tip, t, c);
    // The curve's derivative: 2(1-t)(bend-root) + 2t(tip-bend).
    T.subVectors(TAIL.bend, TAIL.root)
      .multiplyScalar(2 * (1 - t))
      .addScaledVector(next.subVectors(TAIL.tip, TAIL.bend), 2 * t)
      .normalize();
    // The curve lies in the y-z plane, so x is always across it.
    side.copy(x);
    vert.crossVectors(T, side).normalize();
    // Chunky at the root, slim at the fluke, rounded off at the very tip.
    const r = MASCOT_R * (0.36 * (1 - t) + 0.085 * t) * (t > 0.97 ? 0.6 : 1);
    for (let j = 0; j <= sides; j++) {
      const a = (j / sides) * TAU;
      n.copy(side).multiplyScalar(Math.cos(a)).addScaledVector(vert, Math.sin(a));
      pos.push(c.x + n.x * r, c.y + n.y * r * 0.9, c.z + n.z * r);
      nrm.push(n.x, n.y, n.z);
    }
  }
  for (let i = 0; i < rings - 1; i++) {
    for (let j = 0; j < sides; j++) {
      const a = i * (sides + 1) + j;
      const b = a + sides + 1;
      index.push(a, a + 1, b, b, a + 1, b + 1);
    }
  }
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute("normal", new BufferAttribute(new Float32Array(nrm), 3));
  g.setIndex(index);
  // Pivot at the root, so the whole tail wags from where it leaves the body.
  g.translate(-TAIL.root.x, -TAIL.root.y, -TAIL.root.z);
  return g;
}

export function createToyParts(skin: Material, lowQuality: boolean): ToyParts {
  const group = new Group();
  const stalkGeo = buildStalk(lowQuality);
  const blobGeo = new SphereGeometry(1, lowQuality ? 18 : 26, lowQuality ? 12 : 18);

  // ---- Tail -----------------------------------------------------------------
  const tail = new Group();
  tail.position.copy(TAIL.root);
  tail.add(new Mesh(stalkGeo, skin));
  const fluke = new Group();
  fluke.position.subVectors(TAIL.tip, TAIL.root);
  tail.add(fluke);
  // Two plump lobes in a V, turned up at their tips like a toy's.
  for (const dir of [-1, 1]) {
    const holder = new Group();
    const lobe = new Mesh(blobGeo, skin);
    lobe.scale.set(0.46, 0.075, 0.27);
    lobe.position.set(dir * 0.38, 0, -0.08);
    holder.add(lobe);
    holder.rotation.set(-0.15, dir * 0.42, dir * -0.22);
    fluke.add(holder);
  }
  group.add(tail);

  // ---- Flippers -------------------------------------------------------------
  // Low on the sides, a little forward of the middle, pointing down and back.
  const flippers = [-1, 1].map((dir) => {
    const holder = new Group();
    holder.position.set(dir * MASCOT_R * 0.9, -0.5, 0.35 * BODY_LONG);
    const fin = new Mesh(blobGeo, skin);
    fin.scale.set(0.34, 0.075, 0.2);
    fin.position.set(dir * 0.27, 0, -0.05);
    holder.add(fin);
    group.add(holder);
    return { holder, dir };
  });

  return {
    group,
    update(t, swim, still) {
      const s = still ? 0 : swim;
      const p = (t * TAU) / WAG;
      // At rest the tail only sways a little; swimming, it wags.
      tail.rotation.y = (0.05 + 0.17 * s) * Math.sin(p);
      tail.rotation.x = (0.03 + 0.07 * s) * Math.sin(p + 1.1);
      fluke.rotation.x = (0.06 + 0.16 * s) * Math.sin(p - 0.9);
      for (const f of flippers) {
        f.holder.rotation.set(0, f.dir * -0.75, f.dir * (-0.6 - (0.08 + 0.2 * s) * Math.sin(p + 0.5)));
      }
    },
    dispose() {
      stalkGeo.dispose();
      blobGeo.dispose();
    },
  };
}
