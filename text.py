import re
import logging
from pathlib import Path
from urllib.parse import urlparse


class TextDetector:

    def __init__(self, rules_file="antiscam.rules"):

        self.words = []
        self.regex = []
        self.campaign_regex = []

        self.urls = []
        self.allow = []

        self.load_rules(rules_file)


    def load_rules(self, filename):

        current = None

        sections = {
            "WORDS": self.words,
            "REGEX": self.regex,
            "CAMPAIGN": self.campaign_regex,
            "URL": self.urls,
            "ALLOW": self.allow
        }


        with open(filename, "r", encoding="utf-8") as f:

            for line in f:

                line = line.strip()

                if not line or line.startswith("#"):
                    continue


                if line.endswith(":"):

                    current = line[:-1]

                    continue


                if current in sections:

                    sections[current].append(line)


        self.regex = [
            re.compile(
                x,
                re.IGNORECASE
            )
            for x in self.regex
        ]


        self.campaign_regex = [
            re.compile(
                x,
                re.IGNORECASE
            )
            for x in self.campaign_regex
        ]


        logging.info(
            f"Loaded text rules: "
            f"{len(self.words)} words, "
            f"{len(self.regex)} regex, "
            f"{len(self.campaign_regex)} campaign rules"
        )


    def is_allowed_url(self, url):

        for allowed in self.allow:

            if url.startswith(allowed):

                return True

        return False



    def check_markdown_links(self, message):

        pattern = r"\[([^\]]+)\]\((https?://[^\)]+)\)"

        for visible, destination in re.findall(
            pattern,
            message
        ):

            visible_lower = visible.lower()

            fake_brands = [
                "steamcommunity.com",
                "roblox.com",
                "discord.com",
                "discord.gg"
            ]


            for brand in fake_brands:

                if brand in visible_lower:

                    if not self.is_allowed_url(destination):

                        return {
                            "detected": True,
                            "type": "direct",
                            "reason": "fake_visible_link",
                            "match": visible
                        }


        return None



    def scan(self, message):

        text = message.lower()


        for word in self.words:

            if word.lower() in text:

                return {
                    "detected": True,
                    "type": "direct",
                    "reason": "wordlist",
                    "match": word
                }


        for regex in self.regex:

            match = regex.search(message)

            if match:

                return {
                    "detected": True,
                    "type": "direct",
                    "reason": "regex",
                    "match": match.group(0)
                }



        markdown = self.check_markdown_links(
            message
        )

        if markdown:

            return markdown



        for regex in self.campaign_regex:

            match = regex.search(message)

            if match:

                return {
                    "detected": True,
                    "type": "campaign",
                    "campaign": "discord_attachment_scam",
                    "match": match.group(0)
                }



        return {
            "detected": False,
            "type": "none"
        }
