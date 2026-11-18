//! Resources: a font's file or an image's pixels, named by the hash of
//! its bytes and sent once.

use std::collections::HashMap;

use peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};

use crate::bytes::{Reader, Writer};
use crate::{Error, Id, MAX_FONT_BYTES, MAX_IMAGE_SIDE};

/// What a fragment draws with but does not carry.
#[derive(Clone, Debug)]
pub enum Resource {
    /// A font's file (one face of it is named by a glyph run's index).
    Font(Blob<u8>),
    /// An image's pixels, with their size and format.
    Image(ImageData),
}

/// [`Resource::Font`] as [`Resource::kind`] says it.
pub const FONT: u8 = 1;
/// [`Resource::Image`] as [`Resource::kind`] says it.
pub const IMAGE: u8 = 2;

const fn format_tag(format: ImageFormat) -> u8 {
    match format {
        ImageFormat::Bgra8 => 1,
        // RGBA, and any format peniko adds (they are `non_exhaustive`)
        // until this names it.
        _ => 0,
    }
}

const fn alpha_tag(alpha: ImageAlphaType) -> u8 {
    match alpha {
        ImageAlphaType::Alpha => 0,
        ImageAlphaType::AlphaPremultiplied => 1,
    }
}

/// An image's size and format: the head of its bytes as they cross, and
/// part of what it is named by.
fn image_head(image: &ImageData) -> [u8; 10] {
    let mut head = Writer::default();
    head.u8(format_tag(image.format));
    head.u8(alpha_tag(image.alpha_type));
    head.u32(image.width);
    head.u32(image.height);
    let mut out = [0_u8; 10];
    for (to, from) in out.iter_mut().zip(&head.out) {
        *to = *from;
    }
    out
}

impl Resource {
    /// Which it is: [`FONT`] or [`IMAGE`].
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Self::Font(_) => FONT,
            Self::Image(_) => IMAGE,
        }
    }

    /// Its name: the hash of its kind and its bytes as they cross.
    #[must_use]
    pub fn id(&self) -> Id {
        match self {
            Self::Font(font) => Id::of(&[&[FONT], font.data()]),
            Self::Image(image) => Id::of(&[&[IMAGE], &image_head(image), image.data.data()]),
        }
    }

    /// How many bytes it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Font(font) => font.len(),
            Self::Image(image) => image.data.len(),
        }
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Its bytes as they cross (after its kind).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Font(font) => font.data().to_vec(),
            Self::Image(image) => {
                let mut out = image_head(image).to_vec();
                out.extend_from_slice(image.data.data());
                out
            }
        }
    }

    /// A resource of `kind` from the bytes it crossed as, checked.
    ///
    /// # Errors
    ///
    /// A kind it doesn't know, a font past [`MAX_FONT_BYTES`], an image
    /// past [`MAX_IMAGE_SIDE`] or whose bytes are not its size's.
    pub fn from_bytes(kind: u8, bytes: Vec<u8>) -> Result<Self, Error> {
        match kind {
            FONT => {
                if bytes.len() > MAX_FONT_BYTES {
                    return Err(Error::TooLarge(bytes.len(), MAX_FONT_BYTES));
                }
                Ok(Self::Font(Blob::from(bytes)))
            }
            IMAGE => {
                let mut r = Reader::new(&bytes);
                let format = match r.u8()? {
                    0 => ImageFormat::Rgba8,
                    1 => ImageFormat::Bgra8,
                    other => return Err(Error::Unknown("image format", other)),
                };
                let alpha_type = match r.u8()? {
                    0 => ImageAlphaType::Alpha,
                    1 => ImageAlphaType::AlphaPremultiplied,
                    other => return Err(Error::Unknown("image alpha", other)),
                };
                let (width, height) = (r.u32()?, r.u32()?);
                if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
                    return Err(Error::ImageSize(width, height));
                }
                let pixels = r.take(r.left())?;
                if format.size_in_bytes(width, height) != Some(pixels.len()) {
                    return Err(Error::ImageSize(width, height));
                }
                Ok(Self::Image(ImageData {
                    data: Blob::from(pixels.to_vec()),
                    format,
                    alpha_type,
                    width,
                    height,
                }))
            }
            other => Err(Error::Unknown("resource kind", other)),
        }
    }
}

/// The resources one side holds, by name.
///
/// A host puts in only what it was sent and names it itself
/// ([`Self::receive`]), so a name is never the sender's word. What it
/// hands back shares its bytes: every glyph run of one font draws from
/// one blob, which is what a renderer's font cache keys on.
#[derive(Debug, Default)]
pub struct Resources {
    held: HashMap<Id, Resource>,
    bytes: usize,
}

impl Resources {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Holds `resource`, and says its name.
    pub fn insert(&mut self, resource: Resource) -> Id {
        let id = resource.id();
        let size = resource.len();
        if self.held.insert(id, resource).is_none() {
            self.bytes = self.bytes.saturating_add(size);
        }
        id
    }

    /// Holds a resource of `kind` from the bytes it crossed as, and says
    /// its name.
    ///
    /// # Errors
    ///
    /// As [`Resource::from_bytes`].
    pub fn receive(&mut self, kind: u8, bytes: Vec<u8>) -> Result<Id, Error> {
        Ok(self.insert(Resource::from_bytes(kind, bytes)?))
    }

    /// Lets go of `id`: whether it held it.
    pub fn remove(&mut self, id: Id) -> bool {
        self.held.remove(&id).is_some_and(|gone| {
            self.bytes = self.bytes.saturating_sub(gone.len());
            true
        })
    }

    /// Whether it holds `id`.
    #[must_use]
    pub fn holds(&self, id: Id) -> bool {
        self.held.contains_key(&id)
    }

    /// How many bytes of resources it holds.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// The font named `id`.
    ///
    /// # Errors
    ///
    /// It holds no font of that name.
    pub fn font(&self, id: Id) -> Result<Blob<u8>, Error> {
        match self.held.get(&id) {
            Some(Resource::Font(font)) => Ok(font.clone()),
            _ => Err(Error::MissingResource(id)),
        }
    }

    /// The image named `id`.
    ///
    /// # Errors
    ///
    /// It holds no image of that name.
    pub fn image(&self, id: Id) -> Result<ImageData, Error> {
        match self.held.get(&id) {
            Some(Resource::Image(image)) => Ok(image.clone()),
            _ => Err(Error::MissingResource(id)),
        }
    }
}
