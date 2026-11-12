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
        // A tap while the mail sheet is still closing can be lost: wait for
        // it to go, then make sure Departures took.
        XCTAssert(to.waitForNonExistence(timeout: 5), "the mail form didn't close")
        let departures = app.buttons["Departures"]
        for _ in 0..<3 where !departures.isSelected {
            departures.tap()
            _ = departures.wait(for: \.isSelected, toEqual: true, timeout: 2)
        }
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

    func testRoundtableJoinTalkAndLeave() {
        app.tabBars.buttons["Chat"].tap()
        app.buttons["Roundtable"].firstMatch.tap()
        let name = app.textFields["Roundtable name"]
        waitFor(name, 5, "the name field")
        name.tap()
        name.typeText("Evening Table")
        app.buttons["Join here"].tap()
        waitFor(app.staticTexts["Evening Table"], 30, "the table")
        // K7BOT joins and greets the table.
        let hello = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'Good evening all'")).firstMatch
        waitFor(hello, 90, "K7BOT's greeting")
        let box = app.textFields["Say something to the table"]
        box.tap()
        box.typeText("good evening from the iPhone")
        app.buttons["Send to the table"].tap()
        let copy = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'solid copy here'")).firstMatch
        waitFor(copy, 90, "K7BOT's reply at the table")
        app.buttons["Leave"].tap()
        waitFor(app.buttons["Join here"], 30, "leaving the table")
    }

    func testHostRoundtableByCq() {
        app.buttons["Roundtable"].firstMatch.tap()
        let name = app.textFields["Roundtable name"]
        waitFor(name, 5, "the name field")
        name.tap()
        name.typeText("Phone Table")
        app.buttons["Whistle and host the table"].tap()
        // Hosting switches to the table; K7BOT hears the call and joins.
        waitFor(app.staticTexts["you host"], 30, "hosting")
        let hello = app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'Good evening all'")).firstMatch
        waitFor(hello, 150, "K7BOT joining")
        // At the table, nothing on Operate may take the station away.
        app.tabBars.buttons["Operate"].tap()
        let search = app.buttons["Search current band"]
        waitFor(search, 5, "the Search button")
        XCTAssertFalse(search.isEnabled, "Search is off while hosting a Roundtable")
    }

    func testEncryptionShowsCheckCode() {
        app.tabBars.buttons["Settings"].tap()
        let prefer = app.buttons["Prefer"]
        for _ in 0..<6 where !prefer.isHittable { app.swipeUp() }
        prefer.tap()
        // Leaving Off asks to confirm the legal warning.
        let turnOn = app.alerts.buttons["Turn on"]
        waitFor(turnOn, 5, "the legal warning")
        turnOn.tap()
        app.buttons["Save"].tap()
        app.tabBars.buttons["Operate"].tap()
        let call = app.textFields["Callsign"]
        call.tap()
        call.typeText("K7BOT")
        app.buttons["Connect"].firstMatch.tap()
        waitFor(app.staticTexts["Linked with K7BOT"], 60, "the link")
        // K7BOT prefers encryption too: the link is sealed and shows its check code.
        waitFor(app.staticTexts["Encrypted"], 120, "encryption")
        let code = app.staticTexts.containing(NSPredicate(format: "label BEGINSWITH 'check code'")).firstMatch
        waitFor(code, 5, "the check code")
    }
}
