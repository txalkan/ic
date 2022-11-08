from typing import List
from typing import Optional

from data_source.commit_type import CommitType
from data_source.finding_data_source import FindingDataSource
from model.finding import Finding
from model.user import User


class DummyFindingDataSource(FindingDataSource):
    def get_open_finding(
        self, repository: str, scanner: str, dependency_id: str, dependency_version: str
    ) -> Optional[Finding]:
        return None

    def commit_has_block_exception(self, commit_type: CommitType, commit_hash: str) -> bool:
        return True

    def create_or_update_open_finding(self, finding: Finding):
        pass

    def get_risk_assessor(self) -> List[User]:
        return []
