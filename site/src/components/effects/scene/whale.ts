// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import {
  BufferAttribute,
  BufferGeometry,
  Group,
  Mesh,
  SphereGeometry,
  Vector3,
  type Material,
} from "three";
import { MASCOT_R } from "./scene-config";

/**
 * WEB-5 O8: the whale under the head. Above the water the visitor only ever
 * sees the head (the mark's dome); when the camera dives, this is the body
 * the head belongs to: the "oh, it's a whale" moment of the D14 dive, and
 * the guide the assistant section names.
 *
 * Procedural and low-poly, in the head's own space (the waterline is y 0,
 * the face looks along +z):
 * - the body is a tube of rings along a spine that leaves the throat
 *   straight back, then curls to the left within its first 40 % (a whale
 *   turning towards you), so the camera, which dives in front of the head,
 *   sees the whole body side-on running across the screen towards the
 *   middle (on desktop the creature sits right of centre);
 *   it is a little wider than tall, round at the throat, full at the chest,
 *   tapering to a slim tail stock;
 * - the spine undulates slowly, more towards the tail (a whale swims by
 *   beating its fluke up and down), and the fluke follows the tail's slope;
 * - two pectoral flippers and the two lobes of the fluke are flattened
 *   ellipsoids, rocking gently;
 * - the whole body stays below the surface: behind the head its back is
 *   under the waterline (0.06 at the shoulder, 0.3 at the chest), so above
 *   the water you still see only the head; the scene also draws it only
 *   in the picture below the water.
 * The rings are rebuilt on the CPU (about 1,000 vertices) only for frames
 * that draw the picture below the water.
 */

export type Whale = {
  group: Group;
  /** Rebuild the pose for time `t`. Cheap enough per frame; skipped while nothing under water is drawn. */
  update: (t: number, still: boolean) => void;
  dispose: () => void;
};

const TAU = Math.PI * 2;
/** Body length along its spine, in world units (the head's waterline radius is 1.35). */
const LENGTH = 8.6;
/** One fluke beat, in seconds: slow and calm. */
const BEAT = 4.6;

const smooth = (a: number, b: number, x: number): number => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

/** Body radius along the spine (0 the throat, 1 the tail stock). */
function radius(s: number): number {
  const nose = Math.sqrt(Math.min(1, s / 0.07));
  const chest = 0.95 + 0.15 * smooth(0, 0.18, s);
  const taper = Math.pow(1 - smooth(0.2, 0.97, s), 0.9);
  return MASCOT_R * (nose * (chest * taper + 0.1 * (1 - taper)) * (s < 0.995 ? 1 : 0.2));
}

export function createWhale(skin: Material, lowQuality: boolean): Whale {
  const group = new Group();
  const RINGS = lowQuality ? 30 : 44;
  const SIDES = lowQuality ? 18 : 26;
  const count = RINGS * (SIDES + 1);
  const pos = new Float32Array(count * 3);
  const nrm = new Float32Array(count * 3);
  const index: number[] = [];
  for (let i = 0; i < RINGS - 1; i++) {
    for (let j = 0; j < SIDES; j++) {
      const a = i * (SIDES + 1) + j;
      const b = a + SIDES + 1;
      index.push(a, a + 1, b, b, a + 1, b + 1);
    }
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(pos, 3));
  geometry.setAttribute("normal", new BufferAttribute(nrm, 3));
  geometry.setIndex(index);
  const body = new Mesh(geometry, skin);
  // The pose moves every frame; a fixed bound would cull it wrongly.
  body.frustumCulled = false;
  group.add(body);

  // Flippers and flukes: one squashed sphere, scaled per part.
  const partGeo = new SphereGeometry(1, lowQuality ? 14 : 20, lowQuality ? 10 : 14);
  const part = (sx: number, sy: number, sz: number) => {
    const holder = new Group();
    const mesh = new Mesh(partGeo, skin);
    mesh.scale.set(sx, sy, sz);
    // Pivot at the root: the part extends along its own +z from the holder,
    // which `lookAt` below turns to face back along the body.
    mesh.position.z = sz * 0.92;
    holder.add(mesh);
    group.add(holder);
    return holder;
  };
  const flippers = [part(0.3, 0.075, 0.85), part(0.3, 0.075, 0.85)];
  const flukes = [part(0.5, 0.075, 1.15), part(0.5, 0.075, 1.15)];

  // ---- The spine -----------------------------------------------------------
  const spine = Array.from({ length: RINGS }, () => new Vector3());
  const tangent = Array.from({ length: RINGS }, () => new Vector3());
  const up = new Vector3(0, 1, 0);
  const side = new Vector3();
  const vert = new Vector3();
  const tmp = new Vector3();

  function pose(t: number, still: boolean): void {
    const phase = still ? 0.9 : (t * TAU) / BEAT;
    const ds = LENGTH / (RINGS - 1);
    // Integrate the heading: straight back from the throat, then a slow
    // turn to the left; the height sinks gently towards the tail.
    let x = 0;
    // The throat starts under the front of the head, like a lower jaw.
    let z = 1.05;
    for (let i = 0; i < RINGS; i++) {
      const s = i / (RINGS - 1);
      const heading = 1.3 * smooth(0, 0.42, s) + 0.12 * s;
      if (i > 0) {
        x -= Math.sin(heading) * ds;
        z -= Math.cos(heading) * ds;
      }
      const amp = 0.03 + 0.32 * Math.pow(s, 2.1);
      // Sinks quickly behind the throat, so the back stays under the surface.
      const y = -1.15 - 0.95 * smooth(0, 0.35, s) - 0.35 * s + amp * Math.sin(phase - TAU * 0.85 * s);
      spine[i].set(x, y, z);
    }
    for (let i = 0; i < RINGS; i++) {
      const a = spine[Math.max(0, i - 1)];
      const b = spine[Math.min(RINGS - 1, i + 1)];
      tangent[i].subVectors(b, a).normalize();
    }

    // Rings: a little wider than tall, and flatter underneath.
    let k = 0;
    for (let i = 0; i < RINGS; i++) {
      const s = i / (RINGS - 1);
      const r = radius(s);
      const dr = (radius(Math.min(1, s + 0.01)) - radius(Math.max(0, s - 0.01))) / (0.02 * LENGTH);
      const T = tangent[i];
      side.crossVectors(up, T).normalize();
      vert.crossVectors(T, side).normalize();
      const rx = r * 1.1;
      for (let j = 0; j <= SIDES; j++) {
        const a = (j / SIDES) * TAU;
        const c = Math.cos(a);
        const sn = Math.sin(a);
        const ry = r * (sn < 0 ? 0.82 : 0.95);
        const o = k * 3;
        pos[o] = spine[i].x + side.x * c * rx + vert.x * sn * ry;
        pos[o + 1] = spine[i].y + side.y * c * rx + vert.y * sn * ry;
        pos[o + 2] = spine[i].z + side.z * c * rx + vert.z * sn * ry;
        tmp
          .copy(side)
          .multiplyScalar(c / Math.max(rx, 1e-3))
          .addScaledVector(vert, sn / Math.max(ry, 1e-3))
          .normalize()
          .addScaledVector(T, -dr)
          .normalize();
        nrm[o] = tmp.x;
        nrm[o + 1] = tmp.y;
        nrm[o + 2] = tmp.z;
        k++;
      }
    }
    geometry.attributes.position.needsUpdate = true;
    geometry.attributes.normal.needsUpdate = true;

    // Flippers: from the chest's sides, swept back and down, rocking.
    const fi = Math.round(0.2 * (RINGS - 1));
    const fr = radius(0.2);
    const T = tangent[fi];
    side.crossVectors(up, T).normalize();
    for (let n = 0; n < 2; n++) {
      const dir = n === 0 ? 1 : -1;
      const f = flippers[n];
      f.position.copy(spine[fi]).addScaledVector(side, dir * fr * 0.92).addScaledVector(up, -fr * 0.42);
      f.rotation.set(0, 0, 0);
      f.lookAt(tmp.copy(f.position).sub(T));
      f.rotateY(dir * -0.7);
      f.rotateZ(dir * (0.95 + 0.12 * Math.sin(phase + 0.6)));
      f.rotateX(0.25);
    }

    // Flukes: at the tail's end, level with the body, pitched by its slope
    // plus a little more, so the beat reads at the very tip.
    const last = spine[RINGS - 1];
    const TL = tangent[RINGS - 1];
    const beat = 0.35 * Math.cos(phase - TAU * 0.85);
    for (let n = 0; n < 2; n++) {
      const dir = n === 0 ? 1 : -1;
      const f = flukes[n];
      f.position.copy(last);
      f.rotation.set(0, 0, 0);
      f.lookAt(tmp.copy(last).sub(TL));
      f.rotateX(beat);
      f.rotateY(dir * 1.0);
    }
  }

  return {
    group,
    update: pose,
    dispose() {
      geometry.dispose();
      partGeo.dispose();
    },
  };
}
