import hashlib
import imagehash
import threading
import traceback
import logging

from PIL import Image
from io import BytesIO
from collections import OrderedDict


DEBUG_USER_ID = 1326970244673310730

DETECTION_THRESHOLD = 0.72
PHASH_PREFILTER = 0.50

HASH_BITS = 64



class ScamDetector:

    __slots__ = (
        "images",
        "sha_index",
        "hash_cache",
        "cache_limit",
        "cache_lock"
    )


    def __init__(self, database):

        self.images = []
        self.sha_index = {}

        self.hash_cache = OrderedDict()
        self.cache_limit = 5000

        self.cache_lock = threading.Lock()


        for image in database["images"]:

            self.images.append(
                (
                    image["filename"],
                    int(image["phash"], 16),
                    int(image["dhash"], 16),
                    int(image["ahash"], 16),
                    image["campaign_id"]
                )
            )


            self.sha_index[image["sha256"]] = {
                "filename": image["filename"],
                "campaign": image["campaign_id"]
            }


        self.images = tuple(self.images)



    def sha256(self, data):

        return hashlib.sha256(data).hexdigest()



    def normalize_image(self, image):

        image = image.convert(
            "RGB"
        )

        image.thumbnail(
            (512, 512),
            Image.Resampling.BICUBIC
        )


        canvas = Image.new(
            "RGB",
            (512, 512)
        )


        canvas.paste(
            image,
            (
                (512 - image.width) // 2,
                (512 - image.height) // 2
            )
        )


        return canvas



    def hash_to_int(self, value):

        return int(
            str(value),
            16
        )



    def hash_similarity(self, first, second):

        distance = (
            first ^ second
        ).bit_count()

        return 1 - (
            distance / HASH_BITS
        )



    def generate_hashes(self, image):

        return (
            self.hash_to_int(
                imagehash.phash(image)
            ),

            self.hash_to_int(
                imagehash.dhash(image)
            ),

            self.hash_to_int(
                imagehash.average_hash(image)
            )
        )



    def scan(self, image_bytes, user_id=None):

        try:

            sha = self.sha256(
                image_bytes
            )


            exact = self.sha_index.get(
                sha
            )


            if exact:

                return {
                    "confidence": 1.0,
                    "campaign": exact["campaign"],
                    "matches": [
                        {
                            "filename": exact["filename"],
                            "campaign": exact["campaign"],
                            "confidence": 1.0
                        }
                    ]
                }



            with self.cache_lock:

                cached = self.hash_cache.get(
                    sha
                )


            if cached:

                phash, dhash, ahash = cached


                with self.cache_lock:

                    self.hash_cache.move_to_end(
                        sha
                    )


            else:

                try:

                    image = Image.open(
                        BytesIO(image_bytes)
                    )


                    normalized = self.normalize_image(
                        image
                    )


                    phash, dhash, ahash = self.generate_hashes(
                        normalized
                    )


                    with self.cache_lock:

                        self.hash_cache[sha] = (
                            phash,
                            dhash,
                            ahash
                        )


                        if len(self.hash_cache) > self.cache_limit:

                            self.hash_cache.popitem(
                                last=False
                            )


                except Exception:

                    logging.error(
                        traceback.format_exc()
                    )

                    return None



            matches = []

            highest_debug = None



            for (
                filename,
                known_phash,
                known_dhash,
                known_ahash,
                campaign
            ) in self.images:


                p = self.hash_similarity(
                    phash,
                    known_phash
                )


                if p < PHASH_PREFILTER:

                    continue



                d = self.hash_similarity(
                    dhash,
                    known_dhash
                )


                a = self.hash_similarity(
                    ahash,
                    known_ahash
                )


                confidence = (
                    p * 0.50 +
                    d * 0.35 +
                    a * 0.15
                )



                if (
                    highest_debug is None
                    or confidence > highest_debug["confidence"]
                ):

                    highest_debug = {

                        "filename": filename,

                        "confidence": confidence,

                        "phash": hex(known_phash),

                        "dhash": hex(known_dhash),

                        "ahash": hex(known_ahash)

                    }



                if confidence >= DETECTION_THRESHOLD:

                    matches.append(
                        {
                            "filename": filename,
                            "campaign": campaign,
                            "confidence": confidence
                        }
                    )



            if (
                user_id == DEBUG_USER_ID
                and highest_debug
            ):

                print(
                    f"""
=========================
Aegis Debug Scan

Best Match:
{highest_debug["filename"]}

Confidence:
{highest_debug["confidence"]:.2%}

Database pHash:
{highest_debug["phash"]}

Database dHash:
{highest_debug["dhash"]}

Database aHash:
{highest_debug["ahash"]}

=========================
"""
                )



            if not matches:

                return None



            matches.sort(
                key=lambda x: x["confidence"],
                reverse=True
            )


            return {
                "confidence": matches[0]["confidence"],
                "campaign": matches[0]["campaign"],
                "matches": matches
            }


        except Exception:

            logging.error(
                traceback.format_exc()
            )

            return None
