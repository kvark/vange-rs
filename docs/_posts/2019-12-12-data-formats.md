---
layout: post
title: Data Formats
---

The original game features vast levels that were destructible and "live": there was often something moving underground, gates opening and closing, seasons changing, making the world feel very natural. But how could all of this be even stored in memory of a machine from the last century? 128Mb of RAM was supposed to be enough for the whole system running the game.

## Level

Level data is stored and maintained on a per-texel as a multi-layer height map with metadata. Every point of potentially 4096x32768 resolution of a level has its own 1-byte height and 1-byte metadata. For areas that are double-layered, two horizontally adjacent cells are merged to represent the 2x1 segment of the map. They encode the following values:
  - height of the lower layer
  - height of the empty space above the lower layer (the height of the cave)
  - height of the upper layer

![level layers]({{site.baseurl}}/assets/level-layers.png)

The space above the upper layer is always considered empty. The space between the roof of the cave and the upper layer is filled with the ground. Conceptually, this is a 2.5-layer map stored in an interleaved 2x1 segments. It's not 3-layered because we only have metadata for the lower and the higher layers.

Main part of the metadata is the type of the material. Vangers had 8 types, each having different lighting curves. Type 0 is always "water". In addition, there are flags like the double-layer bit, the shadow bit, and others.

The height and metadata are compressed line by line with "splay" algorithm that looks like a regular "deflate" in Zip and others. The level file contains Huffman decoding trees for the bytes and then offsets into each line in the compressed byte stream. The original game only kept those parts in main memory that were visible on the screen, potentially compressing and saving modified data when it goes out of the view. Player was capable of both dynamically destroying the level data and building it.

The height map works best when viewed from the top under narrow angles. This is what the original game allowed to do, demonstrating clear 3D volume without breaking the immersion. Rendering dynamically modified multi-layered height maps is already hard on the current-generation polygonal-optimized hardware. Once the angle becomes more steep, it becomes nearly impossible, without having holes or gaps on the walls, making 1st person camera rather problematic:

![skipped-hills](https://user-images.githubusercontent.com/107301/45591412-0774fe80-b920-11e8-8b5a-0e19f2046ca5.png)

## Model

Items and cars in the game are polygonal, with data stored in ".m3d" files. Each file is designed to represent a full featured car. It has multiple models in it: body of the car, wheels, debris parts. For each, the is a rendering model provided in triangles as well as a simpler collision model provided in quads. The polygonal data is stored in the same way as Wavefront OBJ format specifies:
  - there is a separate array of vertex positions
  - separate array of normals
  - and separate array of polygons, each having indices into vertices and normals, plus some extra data, such as the flat normal

Loading such format into a GPU-friendly representation requires us to build a new index buffer by de-duplicating all the `(vertex, normal)` pairs. Here is one of the cars rendered with collision geometry:

![car-shape]({{site.baseurl}}/assets/model-shape.png)

Interestingly, each of the parts in a model comes with its own definition of the physics parameters:
  - volume of the part
  - 3x3 Jacobian matrix used for collision calculations (see [Collision Model]({{site.baseurl}}/{% post_url 2019-12-17-collision-model %}))

Technically, each part is encoded into its own format, which we can call a "mesh" format. It's just that the developers decided to not store anything in this format directly, and instead preferred to wrap it into ".m3d" even for simple items.

## Parameters

In the original game, parameters are described in a rather free-form way. The confusing part is having 3 different extensions of the files.

Example piece from `escaves.prm`:
```
ZeePa Necross 1024 256 PEREPONKA
POPONKA        B-Zone
none
```

And here is a piece from `game.lst`. Notice a difference?
```
ModelNum 36
Name resource/m3d/mechous/m12.m3d
Size	66
NameID	RiverBier
```

Finally, this is a line from `wrlds.dat`:
```
Necross 	thechain/necross/world.ini
```