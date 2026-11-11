import XCTest

/// Real taps on the app, with the engine's demo station (K7BOT on a
/// simulated band): link, chat, Depot, search.
final class WhistlestopUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchEnvironment["WS_UITEST"] = "1"
        app.launch()
        XCTAssert(app.staticTexts["N0PHN"].waitForExistence(timeout: 10))
        // The modem (demo Mercury) comes up.
        let modem = app.descendants(matching: .any)["Modem ready"]
        XCTAssert(modem.waitForExistence(timeout: 20), "the modem didn't come up")
    }

    private func waitFor(_ e: XCUIElement, _ secs: TimeInterval, _ what: String) {
        XCTAssert(e.waitForExistence(timeout: secs), "timed out waiting for \(what)")
    }

    func testConnectChatAndDisconnect() {
        let call = app.textFields["Callsign"]
        call.tap()
        call.typeText("K7BOT")
        app.buttons["Connect"].firstMatch.tap()
        // Linking switches to Chat.
        waitFor(app.staticTexts["Linked with K7BOT"], 60, "the link")
        let box = app.textFields["Type a message"]
        waitFor(box, 5, "the message box")
        box.tap()
        box.typeText("hello from the iPhone")
        app.buttons["Send"].tap()
        // The demo station answers, quoting us.
        let reply = app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] 'hello from the iPhone' AND NOT (label == 'hello from the iPhone')")).firstMatch
        waitFor(reply, 90, "K7BOT's reply")
        app.buttons["Disconnect"].firstMatch.tap()
        waitFor(app.staticTexts["Not linked"], 60, "the link going down")
    }

    func testWriteDepotMail() {
        app.tabBars.buttons["Depot"].tap()
        app.buttons["Write mail"].tap()
        let to = app.textFields["To (callsign)"]
        waitFor(to, 5, "the mail form")
        to.tap()
        to.typeText("K7BOT")
        let body = app.textViews.firstMatch
        body.tap()
        body.typeText("See you on 20m tonight.")
        app.buttons["Post"].tap()
        app.buttons["Departures"].tap()
        let row = app.descendants(matching: .any).containing(NSPredicate(format: "label CONTAINS 'To K7BOT'")).firstMatch
        waitFor(row, 10, "the mail in Departures")
    }

    func testSearchFindsTheDemoStation() {
        app.buttons["Search current band"].tap()
        waitFor(app.buttons["Stop search"], 5, "the search to start")
        // Search locks onto K7BOT's whistle, then its CQ is decoded.
        let answer = app.buttons["Answer"].firstMatch
        waitFor(answer, 150, "K7BOT in Heard")
        answer.tap()
        waitFor(app.staticTexts["Linked with K7BOT"], 90, "the link")
    }

    func testHelpModeExplains() {
        app.buttons["Help mode"].tap()
        let q = app.buttons["Help: Search"]
        waitFor(q, 5, "the help button")
        q.tap()
        waitFor(app.staticTexts["Search"].firstMatch, 5, "the explanation")
    }
}
