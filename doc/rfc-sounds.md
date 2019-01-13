# Sound support in `vange-rs`

The following RFC is actual at January of 2019.

## Contents

* [Implementation requirements](#implementation-requirements)
* [Libraries for Rust](#libraries-for-rust)
* [Implementation design](#implementation-design)
  * [Configuration options](#configuration-options)
  * [Sound API](#sound-api)

* * *


## Implementation requirements

* The sounds must work in Windows environment
* The sounds must work in Linux environment
* The WAV format must be supported
* The OGG format must be supported


## Libraries for Rust


One of the considerations during research was 3rd party software
licensing to ensure compatibility.

I've done a little research on libraries and found the following:

* [alto](https://crates.io/crates/alto) - This is an alive crate to
work with [OpenAL](https://repo.or.cz/openal-soft.git) library. The
[OpenAL](https://repo.or.cz/openal-soft.git) license currently is GPLv2
in the sources but the website says it's LGPLv2.
* [ears](https://github.com/jhasse/ears) - This is a library built on
top of [OpenAL](https://repo.or.cz/openal-soft.git) and
[libsndfile](http://www.mega-nerd.com/libsndfile/) (LGPLv3-licensed).
Alive and maintained.
* [rust-portaudio](https://github.com/RustAudio/rust-portaudio) -
bindings for [portaudio](http://www.portaudio.com/) library which is
cross-platform API to allow playing music on Windows, Mac OS X, Linux
(ALSA) and unices with OSS (Open Sound System).
[portaudio](http://www.portaudio.com/) is licensed under MIT license.
* [rust-jack](https://github.com/RustAudio/rust-jack) - Bindings for
[JACK Audio Connection Kit](http://jackaudio.org/).
* [rodio](https://github.com/tomaka/rodio) - This is a high-level
library backed by [cpal](https://github.com/tomaka/cpal).

It is said that [OpenAL](https://repo.or.cz/openal-soft.git) supports
the following backends:

* PulseAudio
* ALSA
* OSS
* MMDevAPI
* DirectSound
* CoreAudio
* Solaris
* QSA
* SoundIO
* OpenSL
* WinMM
* PortAudio
* "Null"

## Implementation design

I've decided that [rust-jack](https://github.com/RustAudio/rust-jack)
won't fit our needs because it requires installation of a separate
audio server.

[rust-portaudio](https://github.com/RustAudio/rust-portaudio) looks
better than libraries using [OpenAL](https://repo.or.cz/openal-soft.git)
because [OpenAL](https://repo.or.cz/openal-soft.git) has virus license
and [portaudio](http://www.portaudio.com/) is MIT-licensed.
[OpenAL](https://repo.or.cz/openal-soft.git) also has a dead website
at the moment and the situation looks bad.

[rodio](https://github.com/tomaka/rodio) looks better than the
competitors because it's dual-licensed under Apache 2.0 and MIT without
any viruses and it strives to be pure-Rust library but it does not offer
such portability.

One of my main considerations was portability because I'm using `FreeBSD`
and I was unsure if [cpal](https://github.com/tomaka/cpal) supports
`FreeBSD`. I looked in its `Cargo.toml` and found linking with
`alsa_sys` for BSD flavors which confused me. I had to run *examples*
from its `examples` directory to ensure everything is working fine.

**Conclusion**: [rodio](https://github.com/tomaka/rodio)+[cpal](https://github.com/tomaka/cpal)
is mature and portable enough to continue with it.


### Configuration options


### Sound API



