import XCTest
import DeskDomain

final class BootstrapTests: XCTestCase {
    func testDeskDomainModuleLoads() {
        XCTAssertNotNil(DeskDomainBootstrap.self)
    }
}
