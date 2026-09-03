import hashlib
import imagehash
import threading
import traceback
import logging
from PIL import Image
from io import BytesIO
from collections import OrderedDict
DEBUG_USER_ID = 1326970244673310730
DETECTION_THRESHOLD = 0.805
PHASH_PREFILTER = 0.70
MIN_PHASH_SIMILARITY = 0.70
MIN_DHASH_SIMILARITY = 0.70
HASH_BITS = 64
PHASH_WEIGHT = 0.60
DHASH_WEIGHT = 0.40
AHASH_WEIGHT = 0.00
logging.basicConfig(level=logging.INFO,format="%(asctime)s [%(levelname)s] %(message)s")
logger = logging.getLogger("Aegis")
class ScamDetector:
    __slots__ = ("images","sha_index","hash_cache","cache_limit","cache_lock")
    def __init__(self, database_file="database.aegis"):
        self.images = []
        self.sha_index = {}
        self.hash_cache = OrderedDict()
        self.cache_limit = 5000
        self.cache_lock = threading.Lock()
        self.load_database(database_file)
        self.images = tuple(self.images)
    def load_database(self, filename):
        section = None
        try:
            with open(
                filename,
                "r",
                encoding="utf-8"
            ) as f:
                for line_number, line in enumerate(
                    f,
                    start=1
                ):
                    line = line.strip()
                    if not line:
                        continue
                    if line.startswith("#"):
                        continue
                    if line == "[RULES]":
                        section = "RULES"
                        continue
                    if line == "[HASHES]":
                        section = "HASHES"
                        continue
                    if section != "HASHES":
                        continue
                    parts = line.split()
                    if len(parts) < 5:
                        logger.warning(
                            f"Skipping malformed database "
                            f"line {line_number}"
                        )
                        continue
                    sha256 = parts[0]
                    phash = parts[1]
                    dhash = parts[2]
                    ahash = parts[3]
                    filename = " ".join(
                        parts[4:]
                    )
                    try:
                        self.images.append(
                            (
                                filename,
                                int(phash, 16),
                                int(dhash, 16),
                                int(ahash, 16),
                                "unknown"
                            )
                        )
                        self.sha_index[sha256] = {
                            "filename": filename,
                            "campaign": "unknown"
                        }
                    except ValueError:
                        logger.warning(
                            f"Invalid hash on database "
                            f"line {line_number}: {filename}"
                        )
            logger.info(
                f"Loaded {len(self.images)} image hashes"
            )
            logger.info(
                f"Loaded {len(self.sha_index)} "
                f"SHA-256 entries"
            )
        except Exception:
            logger.error(
                "Failed to load database:"
            )
            logger.error(
                traceback.format_exc()
            )
            raise
    def sha256(self, data):
        return hashlib.sha256(
            data
        ).hexdigest()
    def normalize_image(self, image):
        image = image.convert(
            "RGB"
        )
        image.thumbnail(
            (512, 512),
            Image.Resampling.BICUBIC
        )
        return image
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
        phash = self.hash_to_int(
            imagehash.phash(image)
        )
        dhash = self.hash_to_int(
            imagehash.dhash(image)
        )
        ahash = self.hash_to_int(
            imagehash.average_hash(image)
        )
        return (
            phash,
            dhash,
            ahash
        )
    def calculate_confidence(self, p, d, a):
        return (
            p * PHASH_WEIGHT
            + d * DHASH_WEIGHT
            + a * AHASH_WEIGHT
        )
    def scan(self, image_bytes, user_id=None):
        try:
            if not image_bytes:
                logger.warning("Scan aborted: image_bytes is empty.")
                return None
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
                    with Image.open(
                        BytesIO(image_bytes)
                    ) as image:
                        image.load()
                        normalized = (
                            self.normalize_image(
                                image
                            )
                        )
                        phash, dhash, ahash = (
                            self.generate_hashes(
                                normalized
                            )
                        )
                    with self.cache_lock:
                        self.hash_cache[sha] = (
                            phash,
                            dhash,
                            ahash
                        )
                        if (
                            len(self.hash_cache)
                            > self.cache_limit
                        ):
                            self.hash_cache.popitem(
                                last=False
                            )
                except Exception as error:
                    logger.error(
                        f"Image processing failed: "
                        f"{type(error).__name__}: {error}"
                    )
                    logger.error(
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
                    self.calculate_confidence(
                        p,
                        d,
                        a
                    )
                )
                if (
                    highest_debug is None
                    or confidence
                    > highest_debug["confidence"]
                ):
                    highest_debug = {
                        "filename": filename,
                        "campaign": campaign,
                        "confidence": confidence,
                        "phash_similarity": p,
                        "dhash_similarity": d,
                        "ahash_similarity": a,
                        "phash": hex(
                            known_phash
                        ),
                        "dhash": hex(
                            known_dhash
                        ),
                        "ahash": hex(
                            known_ahash
                        )
                    }
                if (
                    confidence
                    >= DETECTION_THRESHOLD
                    and p
                    >= MIN_PHASH_SIMILARITY
                    and d
                    >= MIN_DHASH_SIMILARITY
                ):
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
pHash similarity:
{highest_debug["phash_similarity"]:.2%}
dHash similarity:
{highest_debug["dhash_similarity"]:.2%}
aHash similarity:
{highest_debug["ahash_similarity"]:.2%}
Database pHash:
{highest_debug["phash"]}
Database dHash:
{highest_debug["dhash"]}
Database aHash:
{highest_debug["ahash"]}
=========================
""")
            if not matches:
                if highest_debug:
                    logger.info(
                        f"No match passed final thresholds. "
                        f"Best candidate: "
                        f"{highest_debug['filename']}, "
                        f"confidence: "
                        f"{highest_debug['confidence']:.2%}"
                    )

                    return {
                        "error": "NO_MATCH",
                        "message": (
                            "Database candidates were found, "
                            "but none passed the final thresholds."
                        ),
                        "best_candidate": highest_debug,
                        "thresholds": {
                            "detection_threshold":
                                DETECTION_THRESHOLD,
                            "phash_prefilter":
                                PHASH_PREFILTER,
                            "min_phash_similarity":
                                MIN_PHASH_SIMILARITY,
                            "min_dhash_similarity":
                                MIN_DHASH_SIMILARITY
                        }
                    }

                else:
                    logger.info(
                        "No database candidates survived "
                        "the pHash prefilter."
                    )

                    return {
                        "error": "NO_PHASH_CANDIDATE",
                        "message": (
                            "No database entry survived "
                            "the pHash prefilter."
                        ),
                        "thresholds": {
                            "phash_prefilter":
                                PHASH_PREFILTER
                        }
                    }
            matches.sort(
                key=lambda x: x["confidence"],
                reverse=True
            )
            return {
                "confidence": matches[0]["confidence"],
                "campaign": matches[0]["campaign"],
                "matches": matches
            }
        except Exception as error:
            logger.error(
                f"Unexpected error during scan: "
                f"{type(error).__name__}: {error}"
            )
            logger.error(
                traceback.format_exc()
            )
            return None
